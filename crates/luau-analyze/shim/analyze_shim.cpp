#include "Luau/Ast.h"
#include "Luau/BuiltinDefinitions.h"
#include "Luau/Config.h"
#include "Luau/ConfigResolver.h"
#include "Luau/Error.h"
#include "Luau/FileResolver.h"
#include "Luau/Frontend.h"

#include <algorithm>
#include <cstdint>
#include <exception>
#include <memory>
#include <optional>
#include <string>
#include <unordered_map>
#include <utility>
#include <vector>

extern "C" {

struct LuauDiagnostic
{
    uint32_t line;
    uint32_t col;
    uint32_t end_line;
    uint32_t end_col;
    uint32_t severity; // 0: error, 1: warning
    const char* message;
    uint32_t message_len;
};

struct LuauCheckResult
{
    void* _internal;
    const LuauDiagnostic* diagnostics;
    uint32_t diagnostic_count;
    uint32_t timed_out;
    uint32_t cancelled;
};

struct LuauString
{
    void* _internal;
    const char* data;
    uint32_t len;
};

typedef struct LuauCancellationToken LuauCancellationToken;

struct LuauCheckOptions
{
    const char* module_name;
    uint32_t module_name_len;
    uint32_t has_timeout;
    double timeout_seconds;
    LuauCancellationToken* cancellation_token;
};

typedef struct LuauChecker LuauChecker;

LuauChecker* luau_checker_new(void);
void luau_checker_free(LuauChecker* checker);

LuauCancellationToken* luau_cancellation_token_new(void);
void luau_cancellation_token_free(LuauCancellationToken* token);
void luau_cancellation_token_cancel(LuauCancellationToken* token);
void luau_cancellation_token_reset(LuauCancellationToken* token);

LuauString luau_checker_add_definitions(
    LuauChecker* checker,
    const char* defs,
    uint32_t defs_len,
    const char* module_name,
    uint32_t module_name_len
);
LuauCheckResult luau_checker_check(
    LuauChecker* checker,
    const char* source,
    uint32_t source_len,
    const LuauCheckOptions* options
);

void luau_check_result_free(LuauCheckResult result);
void luau_string_free(LuauString value);
}

namespace
{

uint32_t as_u32(size_t value)
{
    return value > UINT32_MAX ? UINT32_MAX : static_cast<uint32_t>(value);
}

struct InMemoryFileResolver : Luau::FileResolver
{
    std::unordered_map<Luau::ModuleName, std::string> sources;

    std::optional<Luau::SourceCode> readSource(const Luau::ModuleName& name) override
    {
        auto iterator = sources.find(name);
        if (iterator == sources.end())
            return std::nullopt;
        return Luau::SourceCode{iterator->second, Luau::SourceCode::Module};
    }

    std::optional<Luau::ModuleInfo> resolveModule(const Luau::ModuleInfo*, Luau::AstExpr* expr, const Luau::TypeCheckLimits&) override
    {
        if (const auto* global = expr->as<Luau::AstExprGlobal>())
            return Luau::ModuleInfo{global->name.value};
        if (const auto* string = expr->as<Luau::AstExprConstantString>())
            return Luau::ModuleInfo{std::string(string->value.data, string->value.size)};
        return std::nullopt;
    }

    std::string getHumanReadableModuleName(const Luau::ModuleName& name) const override
    {
        return name;
    }

    std::optional<std::string> getEnvironmentForModule(const Luau::ModuleName&) const override
    {
        return std::nullopt;
    }
};

struct StrictConfigResolver : Luau::ConfigResolver
{
    StrictConfigResolver()
    {
        strictConfig.mode = Luau::Mode::Strict;
    }

    const Luau::Config& getConfig(const Luau::ModuleName&, const Luau::TypeCheckLimits&) const override
    {
        return strictConfig;
    }

    Luau::Config strictConfig;
};

struct DiagnosticEntry
{
    LuauDiagnostic diagnostic{};
    std::string message;
};

struct CheckResultStorage
{
    std::vector<DiagnosticEntry> entries;
    std::vector<LuauDiagnostic> diagnostics;
    bool timedOut = false;
    bool cancelled = false;
};

struct StringStorage
{
    std::string value;
};

struct CheckerImpl
{
    InMemoryFileResolver fileResolver;
    StrictConfigResolver configResolver;
    Luau::FrontendOptions frontendOptions;
    std::unique_ptr<Luau::Frontend> frontend;

    CheckerImpl()
    {
        frontendOptions.retainFullTypeGraphs = false;
        frontendOptions.runLintChecks = true;
        frontend = std::make_unique<Luau::Frontend>(&fileResolver, &configResolver, frontendOptions);
        frontend->setLuauSolverMode(Luau::SolverMode::New);

        Luau::unfreeze(frontend->globals.globalTypes);
        Luau::registerBuiltinGlobals(*frontend, frontend->globals);
        Luau::freeze(frontend->globals.globalTypes);
    }
};

} // namespace

struct LuauChecker
{
    CheckerImpl* impl;
};

struct LuauCancellationToken
{
    std::shared_ptr<Luau::FrontendCancellationToken> token;
};

namespace
{

LuauString make_luau_string(std::string message)
{
    if (message.empty())
        return LuauString{nullptr, nullptr, 0};

    auto* storage = new StringStorage{std::move(message)};
    return LuauString{storage, storage->value.c_str(), as_u32(storage->value.size())};
}

void push_diagnostic(
    CheckResultStorage& storage,
    uint32_t line,
    uint32_t col,
    uint32_t endLine,
    uint32_t endCol,
    uint32_t severity,
    std::string message
)
{
    DiagnosticEntry entry;
    entry.diagnostic.line = line;
    entry.diagnostic.col = col;
    entry.diagnostic.end_line = endLine;
    entry.diagnostic.end_col = endCol;
    entry.diagnostic.severity = severity;
    entry.message = std::move(message);
    storage.entries.push_back(std::move(entry));
}

void push_type_error(CheckResultStorage& storage, const Luau::TypeError& error)
{
    push_diagnostic(
        storage,
        as_u32(error.location.begin.line),
        as_u32(error.location.begin.column),
        as_u32(error.location.end.line),
        as_u32(error.location.end.column),
        0,
        Luau::toString(error)
    );
}

void push_lint(CheckResultStorage& storage, const Luau::LintWarning& warning, uint32_t severity)
{
    push_diagnostic(
        storage,
        as_u32(warning.location.begin.line),
        as_u32(warning.location.begin.column),
        as_u32(warning.location.end.line),
        as_u32(warning.location.end.column),
        severity,
        warning.text
    );
}

void push_internal_error(CheckResultStorage& storage, const std::string& message)
{
    push_diagnostic(storage, 0, 0, 0, 0, 0, message);
}

std::string to_owned_string(const char* data, uint32_t len)
{
    if (data == nullptr || len == 0)
        return {};
    return std::string(data, len);
}

std::string fallback_label(const std::string& value, const char* fallback)
{
    return value.empty() ? std::string(fallback) : value;
}

std::string definitions_error_message(const Luau::LoadDefinitionFileResult& result, const std::string& moduleName)
{
    std::string message;

    for (const auto& parseError : result.parseResult.errors)
    {
        if (!message.empty())
            message += "\n";
        message += moduleName;
        message += ":";
        message += std::to_string(parseError.getLocation().begin.line + 1);
        message += ":";
        message += std::to_string(parseError.getLocation().begin.column + 1);
        message += ": ";
        message += parseError.getMessage();
    }

    if (result.module)
    {
        for (const auto& error : result.module->errors)
        {
            if (!message.empty())
                message += "\n";
            message += moduleName;
            message += ":";
            message += std::to_string(error.location.begin.line + 1);
            message += ":";
            message += std::to_string(error.location.begin.column + 1);
            message += ": ";
            message += Luau::toString(error);
        }
    }

    if (message.empty())
        message = "failed to load Luau definitions";
    return message;
}

LuauCheckResult finalize_check_result(CheckResultStorage* storage)
{
    std::sort(
        storage->entries.begin(),
        storage->entries.end(),
        [](const DiagnosticEntry& left, const DiagnosticEntry& right)
        {
            if (left.diagnostic.line != right.diagnostic.line)
                return left.diagnostic.line < right.diagnostic.line;
            if (left.diagnostic.col != right.diagnostic.col)
                return left.diagnostic.col < right.diagnostic.col;
            if (left.diagnostic.severity != right.diagnostic.severity)
                return left.diagnostic.severity < right.diagnostic.severity;
            return left.message < right.message;
        }
    );

    storage->diagnostics.reserve(storage->entries.size());
    for (auto& entry : storage->entries)
    {
        entry.diagnostic.message = entry.message.c_str();
        entry.diagnostic.message_len = as_u32(entry.message.size());
        storage->diagnostics.push_back(entry.diagnostic);
    }

    return LuauCheckResult{
        storage,
        storage->diagnostics.data(),
        as_u32(storage->diagnostics.size()),
        storage->timedOut ? 1u : 0u,
        storage->cancelled ? 1u : 0u,
    };
}

} // namespace

extern "C" LuauChecker* luau_checker_new(void)
{
    try
    {
        auto* checker = new LuauChecker();
        checker->impl = new CheckerImpl();
        return checker;
    }
    catch (...)
    {
        return nullptr;
    }
}

extern "C" void luau_checker_free(LuauChecker* checker)
{
    if (checker == nullptr)
        return;
    delete checker->impl;
    delete checker;
}

extern "C" LuauCancellationToken* luau_cancellation_token_new(void)
{
    try
    {
        auto* token = new LuauCancellationToken();
        token->token = std::make_shared<Luau::FrontendCancellationToken>();
        token->token->cancelled.store(false);
        return token;
    }
    catch (...)
    {
        return nullptr;
    }
}

extern "C" void luau_cancellation_token_free(LuauCancellationToken* token)
{
    delete token;
}

extern "C" void luau_cancellation_token_cancel(LuauCancellationToken* token)
{
    if (token == nullptr || !token->token)
        return;
    token->token->cancel();
}

extern "C" void luau_cancellation_token_reset(LuauCancellationToken* token)
{
    if (token == nullptr || !token->token)
        return;
    token->token->cancelled.store(false);
}

extern "C" LuauString luau_checker_add_definitions(
    LuauChecker* checker,
    const char* defs,
    uint32_t defs_len,
    const char* module_name,
    uint32_t module_name_len
)
{
    if (checker == nullptr)
        return make_luau_string("checker is null");
    if (defs == nullptr && defs_len > 0)
        return make_luau_string("definitions pointer is null");

    try
    {
        const std::string source = defs == nullptr ? std::string() : std::string(defs, defs_len);
        const std::string moduleName = fallback_label(to_owned_string(module_name, module_name_len), "@definitions");

        Luau::unfreeze(checker->impl->frontend->globals.globalTypes);
        Luau::LoadDefinitionFileResult result = checker->impl->frontend->loadDefinitionFile(
            checker->impl->frontend->globals,
            checker->impl->frontend->globals.globalScope,
            source,
            moduleName,
            false,
            false
        );
        Luau::freeze(checker->impl->frontend->globals.globalTypes);

        if (result.success)
            return LuauString{nullptr, nullptr, 0};
        return make_luau_string(definitions_error_message(result, moduleName));
    }
    catch (const std::exception& error)
    {
        return make_luau_string(error.what());
    }
    catch (...)
    {
        return make_luau_string("unknown error while adding definitions");
    }
}

extern "C" LuauCheckResult luau_checker_check(
    LuauChecker* checker,
    const char* source,
    uint32_t source_len,
    const LuauCheckOptions* options
)
{
    auto* storage = new CheckResultStorage();

    if (checker == nullptr)
    {
        push_internal_error(*storage, "checker is null");
        return finalize_check_result(storage);
    }
    if (source == nullptr && source_len > 0)
    {
        push_internal_error(*storage, "source pointer is null");
        return finalize_check_result(storage);
    }

    try
    {
        std::string moduleName = "main";
        Luau::FrontendOptions frontendOptions = checker->impl->frontendOptions;

        if (options != nullptr)
        {
            moduleName = fallback_label(to_owned_string(options->module_name, options->module_name_len), "main");
            if (options->has_timeout != 0 && options->timeout_seconds >= 0.0)
                frontendOptions.moduleTimeLimitSec = options->timeout_seconds;
            if (options->cancellation_token != nullptr)
                frontendOptions.cancellationToken = options->cancellation_token->token;
        }

        checker->impl->frontend->clear();
        checker->impl->fileResolver.sources.clear();
        checker->impl->fileResolver.sources[moduleName] = source == nullptr ? std::string() : std::string(source, source_len);

        Luau::CheckResult checkResult = checker->impl->frontend->check(moduleName, frontendOptions);
        for (const auto& error : checkResult.errors)
            push_type_error(*storage, error);
        for (const auto& error : checkResult.lintResult.errors)
            push_lint(*storage, error, 0);
        for (const auto& warning : checkResult.lintResult.warnings)
            push_lint(*storage, warning, 1);

        for (const auto& timeoutHit : checkResult.timeoutHits)
        {
            storage->timedOut = true;
            push_internal_error(*storage, "type checking timed out for module `" + timeoutHit + "`");
        }

        if (options != nullptr && options->cancellation_token != nullptr && options->cancellation_token->token &&
            options->cancellation_token->token->requested() && checkResult.errors.empty() &&
            checkResult.lintResult.errors.empty() && checkResult.lintResult.warnings.empty())
        {
            storage->cancelled = true;
            push_internal_error(*storage, "analysis has been cancelled");
        }
        else if (options != nullptr && options->cancellation_token != nullptr && options->cancellation_token->token &&
            options->cancellation_token->token->requested())
            storage->cancelled = true;
    }
    catch (const std::exception& error)
    {
        push_internal_error(*storage, error.what());
    }
    catch (...)
    {
        push_internal_error(*storage, "unknown internal checker error");
    }

    return finalize_check_result(storage);
}

extern "C" void luau_check_result_free(LuauCheckResult result)
{
    delete static_cast<CheckResultStorage*>(result._internal);
}

extern "C" void luau_string_free(LuauString value)
{
    delete static_cast<StringStorage*>(value._internal);
}
