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
#include "llvm/ADT/APSInt.h"
#include "llvm/Support/JSON.h"
#include "llvm/Support/raw_ostream.h"

#include <algorithm>
#include <cstdint>
#include <functional>
#include <memory>
#include <optional>
#include <string>
#include <vector>

namespace {

using llvm::json::Array;
using llvm::json::Object;
using llvm::json::Value;

/// The TIR stores one `i64` per constant. A signed constant keeps its sign, an
/// unsigned constant keeps its value (so `4000000000u` is not sign-extended to
/// a negative number), and an unsigned 64-bit constant that does not fit `i64`
/// keeps its bit pattern, which the translator renders back at its own width.
std::int64_t constant_value(const llvm::APInt &value, bool is_unsigned) {
  return is_unsigned ? static_cast<std::int64_t>(value.getZExtValue())
                     : value.getSExtValue();
}

std::int64_t constant_value(const llvm::APSInt &value) {
  return constant_value(value, value.isUnsigned());
}

/// The exported name of a record or enum: its own name, or the typedef that
/// declares an anonymous one. Empty when the tag has neither, which the type
/// table reports as unknown instead of as a type with no name.
std::string tag_name(const clang::TagDecl *decl) {
  std::string name = decl->getNameAsString();
  if (name.empty()) {
    if (const auto *typedef_decl = decl->getTypedefNameForAnonDecl()) {
      name = typedef_decl->getNameAsString();
    }
  }
  return name;
}

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
  if (const auto *array = context.getAsConstantArrayType(type)) {
    Object node;
    node["kind"] = "array";
    node["elem"] = type_json(array->getElementType(), context);
    node["len"] = array->getSize().getZExtValue();
    return Value(std::move(node));
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
  if (const auto *enum_type = type->getAs<clang::EnumType>()) {
    std::string name = tag_name(enum_type->getDecl());
    if (name.empty()) {
      return Value(scalar("unknown", "anonymous enum"));
    }
    Object node;
    node["kind"] = "enum";
    node["name"] = name;
    return Value(std::move(node));
  }
  if (const auto *record_type = type->getAs<clang::RecordType>()) {
    std::string name = tag_name(record_type->getDecl());
    if (name.empty()) {
      return Value(scalar("unknown", "anonymous record"));
    }
    Object node;
    node["kind"] = "record";
    node["name"] = name;
    return Value(std::move(node));
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

  if (clang::isa<clang::BreakStmt>(stmt)) {
    Object node;
    node["kind"] = "break";
    node["span"] = span_json(stmt->getBeginLoc(), context);
    return node;
  }

  if (clang::isa<clang::ContinueStmt>(stmt)) {
    Object node;
    node["kind"] = "continue";
    node["span"] = span_json(stmt->getBeginLoc(), context);
    return node;
  }

  if (auto *do_stmt = clang::dyn_cast<clang::DoStmt>(stmt)) {
    Object node;
    node["kind"] = "do_while";
    node["body"] = stmt_json(do_stmt->getBody(), context);
    node["cond"] = expr_json(do_stmt->getCond(), context);
    node["span"] = span_json(do_stmt->getBeginLoc(), context);
    return node;
  }

  if (auto *switch_stmt = clang::dyn_cast<clang::SwitchStmt>(stmt)) {
    Object node;
    node["kind"] = "switch";
    node["scrutinee"] = expr_json(switch_stmt->getCond(), context);

    Array cases;
    std::optional<size_t> current;
    bool unattributed = false;

    auto begin_case = [&](std::optional<int64_t> value) {
      Object entry;
      if (value.has_value()) {
        entry["label"] = *value;
      } else {
        entry["label"] = nullptr;
      }
      entry["stmts"] = Array();
      entry["falls_through"] = false;
      cases.push_back(std::move(entry));
      current = cases.size() - 1;
    };

    // A switch body is a list of statements where each case owns the
    // statements that follow it. Nested `case` labels (empty fallthrough) are
    // separate arms with empty bodies. clang keeps only the first statement of
    // an arm under the label and leaves the rest as siblings of it, so a
    // statement belongs to an arm when a label precedes it in the body.
    std::function<void(const clang::Stmt *, bool)> append = [&](const clang::Stmt *child,
                                                               bool inside_case) {
      if (child == nullptr) {
        return;
      }
      if (auto *case_stmt = clang::dyn_cast<clang::CaseStmt>(child)) {
        auto value = case_stmt->getLHS()->EvaluateKnownConstInt(context);
        bool is_unsigned = case_stmt->getLHS()->getType()->isUnsignedIntegerOrEnumerationType();
        begin_case(constant_value(value, is_unsigned));
        append(case_stmt->getSubStmt(), true);
        return;
      }
      if (auto *default_stmt = clang::dyn_cast<clang::DefaultStmt>(child)) {
        begin_case(std::nullopt);
        append(default_stmt->getSubStmt(), true);
        return;
      }
      if (auto *compound = clang::dyn_cast<clang::CompoundStmt>(child)) {
        for (const clang::Stmt *inner : compound->body()) {
          append(inner, inside_case || current.has_value());
        }
        return;
      }
      if (!inside_case || !current.has_value()) {
        // A statement before the first label, or one the arms do not own.
        // Translating the surrounding arms would silently drop it, and
        // document 06 section 10.2 flags such a switch.
        unattributed = true;
        return;
      }
      if (auto *array = cases[*current].getAsObject()->getArray("stmts")) {
        array->push_back(stmt_json(child, context));
      }
    };

    append(switch_stmt->getBody(), false);
    if (unattributed) {
      Object unsupported;
      unsupported["kind"] = "unsupported";
      unsupported["spelling"] = "switch with a statement outside every case";
      unsupported["span"] = span_json(switch_stmt->getBeginLoc(), context);
      return unsupported;
    }

    // A case falls through when its last statement can reach the next case.
    for (size_t index = 0; index + 1 < cases.size(); ++index) {
      auto *entry = cases[index].getAsObject();
      auto *stmts = entry->getArray("stmts");
      bool falls = stmts == nullptr || stmts->empty();
      if (!falls) {
        const Value &last = stmts->back();
        std::optional<llvm::StringRef> kind = last.getAsObject()
                                                 ? last.getAsObject()->getString("kind")
                                                 : std::nullopt;
        // `continue` leaves the switch for the enclosing loop, so the next
        // case is not reached and the arm does not fall through.
        falls = !(kind && (*kind == "break" || *kind == "return" || *kind == "continue"));
      }
      (*entry)["falls_through"] = falls;
    }
    if (!cases.empty()) {
      (*cases.back().getAsObject())["falls_through"] = false;
    }

    node["cases"] = std::move(cases);
    node["span"] = span_json(switch_stmt->getBeginLoc(), context);
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
    bool is_unsigned = expr->getType()->isUnsignedIntegerOrEnumerationType();
    node["value"] = constant_value(literal->getValue(), is_unsigned);
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

  if (auto *literal = clang::dyn_cast<clang::CharacterLiteral>(expr)) {
    Object node;
    node["kind"] = "char_lit";
    node["value"] = literal->getValue();
    node["type"] = type_json(expr->getType(), context);
    return node;
  }

  // `sizeof` is resolved here, against the target the exporter is compiling
  // for, so the frontend never has to model C layout itself.
  if (auto *size = clang::dyn_cast<clang::UnaryExprOrTypeTraitExpr>(expr)) {
    if (size->getKind() == clang::UETT_SizeOf) {
      clang::QualType measured = size->isArgumentType()
                                     ? size->getArgumentType()
                                     : size->getArgumentExpr()->getType();
      Object node;
      node["kind"] = "int_lit";
      node["value"] = static_cast<int64_t>(
          context.getTypeSizeInChars(measured).getQuantity());
      node["type"] = type_json(expr->getType(), context);
      return node;
    }
  }

  if (auto *reference = clang::dyn_cast<clang::DeclRefExpr>(expr)) {
    Object node;
    node["kind"] = "decl_ref";
    node["name"] = reference->getDecl()->getNameAsString();
    node["type"] = type_json(expr->getType(), context);
    return node;
  }

  if (auto *member = clang::dyn_cast<clang::MemberExpr>(expr)) {
    Object node;
    node["kind"] = "member";
    node["base"] = expr_json(member->getBase(), context);
    node["field"] = member->getMemberNameInfo().getAsString();
    node["arrow"] = member->isArrow();
    node["type"] = type_json(expr->getType(), context);
    return node;
  }

  if (auto *conditional = clang::dyn_cast<clang::ConditionalOperator>(expr)) {
    Object node;
    node["kind"] = "cond";
    node["cond"] = expr_json(conditional->getCond(), context);
    node["then_branch"] = expr_json(conditional->getTrueExpr(), context);
    node["else_branch"] = expr_json(conditional->getFalseExpr(), context);
    node["type"] = type_json(expr->getType(), context);
    return node;
  }

  if (auto *list = clang::dyn_cast<clang::InitListExpr>(expr)) {
    Object node;
    node["kind"] = "init_list";
    Array elems;
    for (const clang::Expr *elem : list->inits()) {
      elems.push_back(expr_json(elem, context));
    }
    node["elems"] = std::move(elems);
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
  struct Output {
    std::vector<Object> functions;
    std::vector<Object> records;
    std::vector<Object> enums;
    std::vector<Object> globals;
  };

  AstVisitor(clang::ASTContext &context, Output &output)
      : context(context), output(output) {}

  bool VisitRecordDecl(clang::RecordDecl *record) {
    if (!record->isCompleteDefinition() || record->getDeclContext()->isFunctionOrMethod()) {
      return true;
    }
    auto &source = context.getSourceManager();
    if (!source.isWrittenInMainFile(record->getLocation())) {
      return true;
    }
    // A record declared only through a typedef names itself after the
    // typedef; one that has neither is not representable.
    std::string name = tag_name(record);
    if (name.empty()) {
      return true;
    }
    Object node;
    node["name"] = name;
    clang::QualType self = context.getRecordType(record);
    node["size"] = static_cast<int64_t>(context.getTypeSizeInChars(self).getQuantity());
    node["align"] = static_cast<int64_t>(context.getTypeAlignInChars(self).getQuantity());
    node["span"] = span_json(record->getLocation(), context);
    Array fields;
    for (const clang::FieldDecl *field : record->fields()) {
      Object entry;
      entry["name"] = field->getNameAsString();
      entry["type"] = type_json(field->getType(), context);
      entry["span"] = span_json(field->getLocation(), context);
      fields.push_back(std::move(entry));
    }
    node["fields"] = std::move(fields);
    output.records.push_back(std::move(node));
    return true;
  }

  bool VisitEnumDecl(clang::EnumDecl *item) {
    if (!item->isCompleteDefinition()) {
      return true;
    }
    auto &source = context.getSourceManager();
    if (!source.isWrittenInMainFile(item->getLocation())) {
      return true;
    }
    std::string name = tag_name(item);
    if (name.empty()) {
      return true;
    }
    Object node;
    node["name"] = name;
    node["underlying"] = type_json(item->getIntegerType(), context);
    node["span"] = span_json(item->getLocation(), context);
    Array variants;
    for (const clang::EnumConstantDecl *constant : item->enumerators()) {
      Object entry;
      entry["name"] = constant->getNameAsString();
      entry["value"] = constant_value(constant->getInitVal());
      variants.push_back(std::move(entry));
    }
    node["variants"] = std::move(variants);
    output.enums.push_back(std::move(node));
    return true;
  }

  bool VisitVarDecl(clang::VarDecl *variable) {
    if (!variable->getDeclContext()->isTranslationUnit()) {
      return true;
    }
    auto &source = context.getSourceManager();
    if (!source.isWrittenInMainFile(variable->getLocation())) {
      return true;
    }
    if (variable->getStorageClass() == clang::SC_Extern && !variable->hasInit()) {
      return true;
    }
    Object node;
    node["name"] = variable->getNameAsString();
    node["type"] = type_json(variable->getType(), context);
    node["is_const"] = variable->getType().isConstQualified();
    node["linkage"] = variable->isExternallyVisible() ? "external" : "internal";
    node["span"] = span_json(variable->getLocation(), context);
    if (const clang::Expr *init = variable->getInit()) {
      node["init"] = expr_json(init, context);
    } else {
      node["init"] = nullptr;
    }
    output.globals.push_back(std::move(node));
    return true;
  }

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
    output.functions.push_back(std::move(node));
    return true;
  }

private:
  clang::ASTContext &context;
  Output &output;
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

    AstVisitor::Output output;
    AstVisitor visitor(context, output);
    visitor.TraverseDecl(context.getTranslationUnitDecl());

    Object unit;
    unit["file"] = file;
    auto attach = [&](const char *key, std::vector<Object> &objects) {
      Array array;
      for (Object &object : objects) {
        array.push_back(std::move(object));
      }
      unit[key] = std::move(array);
    };
    attach("functions", output.functions);
    attach("records", output.records);
    attach("enums", output.enums);
    attach("globals", output.globals);
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
