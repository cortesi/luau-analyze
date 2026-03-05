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
};

struct LuauString
{
    void* _internal;
    const char* data;
    uint32_t len;
};

typedef struct LuauChecker LuauChecker;

LuauChecker* luau_checker_new(void);
void luau_checker_free(LuauChecker* checker);

LuauString luau_checker_add_definitions(LuauChecker* checker, const char* defs, uint32_t defs_len);
LuauCheckResult luau_checker_check(LuauChecker* checker, const char* source, uint32_t source_len);

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

std::string definitions_error_message(const Luau::LoadDefinitionFileResult& result)
{
    std::string message;

    for (const auto& parseError : result.parseResult.errors)
    {
        if (!message.empty())
            message += "\n";
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

    return LuauCheckResult{storage, storage->diagnostics.data(), as_u32(storage->diagnostics.size())};
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

extern "C" LuauString luau_checker_add_definitions(LuauChecker* checker, const char* defs, uint32_t defs_len)
{
    if (checker == nullptr)
        return make_luau_string("checker is null");
    if (defs == nullptr && defs_len > 0)
        return make_luau_string("definitions pointer is null");

    try
    {
        const std::string source = defs == nullptr ? std::string() : std::string(defs, defs_len);

        Luau::unfreeze(checker->impl->frontend->globals.globalTypes);
        Luau::LoadDefinitionFileResult result = checker->impl->frontend->loadDefinitionFile(
            checker->impl->frontend->globals,
            checker->impl->frontend->globals.globalScope,
            source,
            "@definitions",
            false,
            false
        );
        Luau::freeze(checker->impl->frontend->globals.globalTypes);

        if (result.success)
            return LuauString{nullptr, nullptr, 0};
        return make_luau_string(definitions_error_message(result));
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

extern "C" LuauCheckResult luau_checker_check(LuauChecker* checker, const char* source, uint32_t source_len)
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
        checker->impl->frontend->clear();
        checker->impl->fileResolver.sources.clear();
        checker->impl->fileResolver.sources["main"] = source == nullptr ? std::string() : std::string(source, source_len);

        Luau::CheckResult checkResult = checker->impl->frontend->check("main");
        for (const auto& error : checkResult.errors)
            push_type_error(*storage, error);
        for (const auto& error : checkResult.lintResult.errors)
            push_lint(*storage, error, 0);
        for (const auto& warning : checkResult.lintResult.warnings)
            push_lint(*storage, warning, 1);
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
