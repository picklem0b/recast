#include "clang/AST/ASTConsumer.h"
#include "clang/AST/ASTContext.h"
#include "clang/AST/Decl.h"
#include "clang/AST/Expr.h"
#include "clang/AST/RecursiveASTVisitor.h"
#include "clang/AST/Stmt.h"
#include "clang/Basic/SourceManager.h"
#include "clang/Basic/TargetInfo.h"
#include "clang/Frontend/CompilerInstance.h"
#include "clang/Frontend/FrontendAction.h"
#include "clang/Tooling/CompilationDatabase.h"
#include "clang/Tooling/JSONCompilationDatabase.h"
#include "clang/Tooling/Tooling.h"
#include "llvm/Support/JSON.h"
#include "llvm/Support/raw_ostream.h"

#include <algorithm>
#include <memory>
#include <optional>
#include <string>
#include <vector>

namespace {

using llvm::json::Array;
using llvm::json::Object;
using llvm::json::Value;

struct AstState {
  std::vector<Object> units;
  std::optional<Object> target;
};

Object scalar(const char *kind, const char *spelling) {
  Object type;
  type["kind"] = kind;
  if (spelling != nullptr) {
    type["spelling"] = spelling;
  }
  return type;
}

Object span_json(clang::SourceLocation location, clang::ASTContext &context) {
  Object span;
  auto &source = context.getSourceManager();
  auto presumed = source.getPresumedLoc(location);
  if (presumed.isInvalid()) {
    span["file"] = "<unknown>";
    span["line"] = 0;
    span["column"] = 0;
    return span;
  }
  span["file"] = presumed.getFilename();
  span["line"] = presumed.getLine();
  span["column"] = presumed.getColumn();
  return span;
}

Value type_json(clang::QualType type, clang::ASTContext &context) {
  type = type.getNonReferenceType();

  if (type->isVoidType()) {
    return Value(scalar("void", nullptr));
  }
  if (type->isBooleanType()) {
    return Value(scalar("bool", nullptr));
  }
  if (type->isPointerType()) {
    Object pointer;
    pointer["kind"] = "pointer";
    clang::QualType pointee = type->getPointeeType();
    pointer["pointee"] = type_json(pointee, context);
    pointer["pointee_const"] = pointee.isConstQualified();
    return Value(std::move(pointer));
  }
  if (auto *typedef_type = type->getAs<clang::TypedefType>()) {
    Object named;
    named["kind"] = "named";
    named["name"] = typedef_type->getDecl()->getNameAsString();
    named["canonical"] =
        type_json(typedef_type->getDecl()->getUnderlyingType(), context);
    return Value(std::move(named));
  }
  if (type->isIntegerType()) {
    Object integer;
    integer["kind"] = "int";
    integer["bits"] = context.getIntWidth(type);
    integer["signed"] = type->isSignedIntegerType();
    integer["spelling"] = type.getAsString();
    return Value(std::move(integer));
  }
  if (type->isFloatingType()) {
    Object floating;
    floating["kind"] = "float";
    floating["bits"] = context.getTypeSize(type);
    floating["spelling"] = type.getAsString();
    return Value(std::move(floating));
  }

  Object unknown;
  unknown["kind"] = "unknown";
  unknown["spelling"] = type.getAsString();
  return Value(std::move(unknown));
}

Object expr_json(const clang::Expr *expr, clang::ASTContext &context);

Object stmt_json(const clang::Stmt *stmt, clang::ASTContext &context) {
  if (stmt == nullptr) {
    Object empty;
    empty["kind"] = "empty";
    return empty;
  }

  if (auto *compound = clang::dyn_cast<clang::CompoundStmt>(stmt)) {
    Object node;
    node["kind"] = "compound";
    Array children;
    for (const clang::Stmt *child : compound->body()) {
      children.push_back(stmt_json(child, context));
    }
    node["stmts"] = std::move(children);
    return node;
  }

  if (auto *decl_stmt = clang::dyn_cast<clang::DeclStmt>(stmt)) {
    Array declarations;
    for (const clang::Decl *decl : decl_stmt->decls()) {
      if (auto *variable = clang::dyn_cast<clang::VarDecl>(decl)) {
        Object node;
        node["kind"] = "decl";
        node["name"] = variable->getNameAsString();
        node["type"] = type_json(variable->getType(), context);
        if (const clang::Expr *init = variable->getInit()) {
          node["init"] = expr_json(init, context);
        } else {
          node["init"] = nullptr;
        }
        node["span"] = span_json(variable->getLocation(), context);
        declarations.push_back(std::move(node));
      }
    }
    if (declarations.size() == 1) {
      return *declarations.begin()->getAsObject();
    }
    Object node;
    node["kind"] = "compound";
    node["stmts"] = std::move(declarations);
    return node;
  }

  if (auto *for_stmt = clang::dyn_cast<clang::ForStmt>(stmt)) {
    Object node;
    node["kind"] = "for";
    if (const clang::Stmt *init = for_stmt->getInit()) {
      node["init"] = stmt_json(init, context);
    } else {
      node["init"] = nullptr;
    }
    if (const clang::Expr *cond = for_stmt->getCond()) {
      node["cond"] = expr_json(cond, context);
    } else {
      node["cond"] = nullptr;
    }
    if (const clang::Expr *step = for_stmt->getInc()) {
      node["step"] = expr_json(step, context);
    } else {
      node["step"] = nullptr;
    }
    node["body"] = stmt_json(for_stmt->getBody(), context);
    node["span"] = span_json(for_stmt->getBeginLoc(), context);
    return node;
  }

  if (auto *while_stmt = clang::dyn_cast<clang::WhileStmt>(stmt)) {
    Object node;
    node["kind"] = "while";
    node["cond"] = expr_json(while_stmt->getCond(), context);
    node["body"] = stmt_json(while_stmt->getBody(), context);
    node["span"] = span_json(while_stmt->getBeginLoc(), context);
    return node;
  }

  if (auto *if_stmt = clang::dyn_cast<clang::IfStmt>(stmt)) {
    Object node;
    node["kind"] = "if";
    node["cond"] = expr_json(if_stmt->getCond(), context);
    node["then_branch"] = stmt_json(if_stmt->getThen(), context);
    if (const clang::Stmt *else_branch = if_stmt->getElse()) {
      node["else_branch"] = stmt_json(else_branch, context);
    } else {
      node["else_branch"] = nullptr;
    }
    node["span"] = span_json(if_stmt->getBeginLoc(), context);
    return node;
  }

  if (auto *return_stmt = clang::dyn_cast<clang::ReturnStmt>(stmt)) {
    Object node;
    node["kind"] = "return";
    if (const clang::Expr *value = return_stmt->getRetValue()) {
      node["value"] = expr_json(value, context);
    } else {
      node["value"] = nullptr;
    }
    node["span"] = span_json(return_stmt->getBeginLoc(), context);
    return node;
  }

  if (clang::isa<clang::NullStmt>(stmt)) {
    Object node;
    node["kind"] = "empty";
    return node;
  }

  if (auto *expr = clang::dyn_cast<clang::Expr>(stmt)) {
    Object node;
    node["kind"] = "expr";
    node["expr"] = expr_json(expr, context);
    node["span"] = span_json(expr->getBeginLoc(), context);
    return node;
  }

  Object node;
  node["kind"] = "unsupported";
  node["spelling"] = stmt->getStmtClassName();
  node["span"] = span_json(stmt->getBeginLoc(), context);
  return node;
}

Object expr_json(const clang::Expr *expr, clang::ASTContext &context) {
  if (auto *literal = clang::dyn_cast<clang::IntegerLiteral>(expr)) {
    Object node;
    node["kind"] = "int_lit";
    node["value"] = literal->getValue().getSExtValue();
    node["type"] = type_json(expr->getType(), context);
    return node;
  }

  if (auto *literal = clang::dyn_cast<clang::FloatingLiteral>(expr)) {
    Object node;
    node["kind"] = "float_lit";
    node["value"] = literal->getValue().convertToDouble();
    node["type"] = type_json(expr->getType(), context);
    return node;
  }

  if (auto *literal = clang::dyn_cast<clang::StringLiteral>(expr)) {
    Object node;
    node["kind"] = "string_lit";
    node["value"] = literal->getBytes().str();
    node["type"] = type_json(expr->getType(), context);
    return node;
  }

  if (auto *reference = clang::dyn_cast<clang::DeclRefExpr>(expr)) {
    Object node;
    node["kind"] = "decl_ref";
    node["name"] = reference->getDecl()->getNameAsString();
    node["type"] = type_json(expr->getType(), context);
    return node;
  }

  if (auto *assign = clang::dyn_cast<clang::CompoundAssignOperator>(expr)) {
    Object node;
    node["kind"] = "assign";
    node["op"] = assign->getOpcodeStr().str();
    node["lhs"] = expr_json(assign->getLHS(), context);
    node["rhs"] = expr_json(assign->getRHS(), context);
    node["type"] = type_json(expr->getType(), context);
    return node;
  }

  if (auto *binary = clang::dyn_cast<clang::BinaryOperator>(expr)) {
    Object node;
    node["kind"] = binary->isAssignmentOp() ? "assign" : "binary";
    node["op"] = binary->getOpcodeStr().str();
    node["lhs"] = expr_json(binary->getLHS(), context);
    node["rhs"] = expr_json(binary->getRHS(), context);
    node["type"] = type_json(expr->getType(), context);
    return node;
  }

  if (auto *unary = clang::dyn_cast<clang::UnaryOperator>(expr)) {
    Object node;
    node["kind"] = "unary";
    node["op"] = clang::UnaryOperator::getOpcodeStr(unary->getOpcode()).str();
    node["operand"] = expr_json(unary->getSubExpr(), context);
    node["postfix"] = unary->isPostfix();
    node["type"] = type_json(expr->getType(), context);
    return node;
  }

  if (auto *index = clang::dyn_cast<clang::ArraySubscriptExpr>(expr)) {
    Object node;
    node["kind"] = "index";
    node["base"] = expr_json(index->getBase(), context);
    node["index"] = expr_json(index->getIdx(), context);
    node["type"] = type_json(expr->getType(), context);
    return node;
  }

  if (auto *call = clang::dyn_cast<clang::CallExpr>(expr)) {
    Object node;
    node["kind"] = "call";
    if (const clang::FunctionDecl *callee = call->getDirectCallee()) {
      node["callee"] = callee->getNameAsString();
    } else {
      node["callee"] = "<indirect>";
    }
    Array args;
    for (const clang::Expr *arg : call->arguments()) {
      args.push_back(expr_json(arg, context));
    }
    node["args"] = std::move(args);
    node["type"] = type_json(expr->getType(), context);
    return node;
  }

  if (auto *cast = clang::dyn_cast<clang::CStyleCastExpr>(expr)) {
    Object node;
    node["kind"] = "cast";
    node["to"] = type_json(cast->getTypeAsWritten(), context);
    node["expr"] = expr_json(cast->getSubExpr(), context);
    node["implicit"] = false;
    node["type"] = type_json(expr->getType(), context);
    return node;
  }

  if (auto *cast = clang::dyn_cast<clang::ImplicitCastExpr>(expr)) {
    Object node;
    node["kind"] = "cast";
    node["to"] = type_json(expr->getType(), context);
    node["expr"] = expr_json(cast->getSubExpr(), context);
    node["implicit"] = true;
    node["type"] = type_json(expr->getType(), context);
    return node;
  }

  if (auto *paren = clang::dyn_cast<clang::ParenExpr>(expr)) {
    Object node;
    node["kind"] = "paren";
    node["expr"] = expr_json(paren->getSubExpr(), context);
    node["type"] = type_json(expr->getType(), context);
    return node;
  }

  Object node;
  node["kind"] = "unsupported";
  node["spelling"] = expr->getStmtClassName();
  node["type"] = type_json(expr->getType(), context);
  return node;
}

class AstVisitor : public clang::RecursiveASTVisitor<AstVisitor> {
public:
  AstVisitor(clang::ASTContext &context, std::vector<Object> &functions)
      : context(context), functions(functions) {}

  bool VisitFunctionDecl(clang::FunctionDecl *function) {
    if (!function->doesThisDeclarationHaveABody()) {
      return true;
    }
    auto &source = context.getSourceManager();
    if (!source.isWrittenInMainFile(function->getLocation())) {
      return true;
    }

    Object node;
    node["name"] = function->getNameAsString();
    node["linkage"] = function->isExternallyVisible() ? "external" : "internal";
    node["return_type"] = type_json(function->getReturnType(), context);

    Array params;
    for (const clang::ParmVarDecl *param : function->parameters()) {
      Object parameter;
      if (!param->getName().empty()) {
        parameter["name"] = param->getNameAsString();
      }
      parameter["type"] = type_json(param->getType(), context);
      params.push_back(std::move(parameter));
    }
    node["params"] = std::move(params);
    node["body"] = stmt_json(function->getBody(), context);
    node["span"] = span_json(function->getLocation(), context);
    functions.push_back(std::move(node));
    return true;
  }

private:
  clang::ASTContext &context;
  std::vector<Object> &functions;
};

Object target_json(clang::ASTContext &context) {
  const clang::TargetInfo &target = context.getTargetInfo();
  std::string triple = target.getTriple().str();

  Object node;
  node["triple"] = triple;
  node["pointer_bits"] = target.getPointerWidth(clang::LangAS::Default);
  node["int_bits"] = target.getIntWidth();
  node["long_bits"] = target.getLongWidth();
  node["long_long_bits"] = target.getLongLongWidth();
  node["char_signed"] = static_cast<bool>(context.getLangOpts().CharIsSigned);
  node["endian"] = target.isBigEndian() ? "big" : "little";
  return node;
}

class AstConsumer : public clang::ASTConsumer {
public:
  AstConsumer(AstState &state, std::string file)
      : state(state), file(std::move(file)) {}

  void HandleTranslationUnit(clang::ASTContext &context) override {
    if (!state.target.has_value()) {
      state.target = target_json(context);
    }

    std::vector<Object> functions;
    AstVisitor visitor(context, functions);
    visitor.TraverseDecl(context.getTranslationUnitDecl());

    Object unit;
    unit["file"] = file;
    Array array;
    for (Object &function : functions) {
      array.push_back(std::move(function));
    }
    unit["functions"] = std::move(array);
    state.units.push_back(std::move(unit));
  }

private:
  AstState &state;
  std::string file;
};

class AstAction : public clang::ASTFrontendAction {
public:
  explicit AstAction(AstState &state) : state(state) {}

  std::unique_ptr<clang::ASTConsumer>
  CreateASTConsumer(clang::CompilerInstance &, llvm::StringRef file) override {
    return std::make_unique<AstConsumer>(state, file.str());
  }

private:
  AstState &state;
};

class AstActionFactory : public clang::tooling::FrontendActionFactory {
public:
  explicit AstActionFactory(AstState &state) : state(state) {}

  std::unique_ptr<clang::FrontendAction> create() override {
    return std::make_unique<AstAction>(state);
  }

private:
  AstState &state;
};

} // namespace

int main(int argc, char **argv) {
  std::string compdb;
  std::string file;

  for (int index = 1; index < argc; ++index) {
    std::string argument = argv[index];
    if (argument == "--compdb" && index + 1 < argc) {
      compdb = argv[++index];
    } else if (argument == "--file" && index + 1 < argc) {
      file = argv[++index];
    } else {
      llvm::errs() << "usage: recast-exporter --compdb <compile_commands.json> "
                      "[--file <source>]\n";
      return 2;
    }
  }

  if (compdb.empty()) {
    llvm::errs() << "recast-exporter: --compdb is required\n";
    return 2;
  }

  std::string error;
  std::unique_ptr<clang::tooling::CompilationDatabase> database =
      clang::tooling::JSONCompilationDatabase::loadFromFile(
          compdb, error, clang::tooling::JSONCommandLineSyntax::AutoDetect);
  if (!database) {
    llvm::errs() << "recast-exporter: " << error << "\n";
    return 3;
  }

  std::vector<std::string> files;
  if (!file.empty()) {
    files.push_back(file);
  } else {
    files = database->getAllFiles();
  }
  if (files.empty()) {
    llvm::errs() << "recast-exporter: no source files in " << compdb << "\n";
    return 3;
  }

  clang::tooling::ClangTool tool(*database, files);
  AstState state;
  AstActionFactory factory(state);
  int result = tool.run(&factory);
  if (result != 0) {
    return result;
  }

  if (!state.target.has_value()) {
    llvm::errs() << "recast-exporter: no translation unit was read\n";
    return 4;
  }

  Object root;
  root["schema"] = "recast-ast/1";
  root["target"] = std::move(*state.target);

  std::sort(state.units.begin(), state.units.end(),
            [](const Object &left, const Object &right) {
              return left.getString("file").value_or("") <
                     right.getString("file").value_or("");
            });

  Array units;
  for (Object &unit : state.units) {
    units.push_back(std::move(unit));
  }
  root["translation_units"] = std::move(units);

  llvm::json::OStream stream(llvm::outs());
  stream.value(Value(std::move(root)));
  llvm::outs() << "\n";
  return 0;
}
