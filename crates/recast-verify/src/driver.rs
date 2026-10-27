//! Driver generation for V2 and property sweeps.
//!
//! Both drivers are produced from the same `Program` and `Case`, so parameter
//! order, record layout and integer widths cannot drift between the two sides.

use std::path::Path;

use recast_ir::{Function, Linkage, Program, Type, TypeId};
use serde_json::Value;

use crate::{Call, Case, Error, Sweep};

/// The C and Rust driver pair.
#[derive(Debug, Clone)]
pub struct Drivers {
    pub c: String,
    pub rust: String,
}

/// Build the call drivers for a whole case.
pub fn calls(program: &Program, case: &Case, source: &Path) -> Result<Drivers, Error> {
    let source = absolute(source);
    let mut c_body = String::new();
    let mut r_body = String::new();
    for call in &case.calls {
        call_driver(program, call, &mut c_body, &mut r_body)?;
    }
    let mut c = String::new();
    c.push_str(&format!(
        "#include \"{}\"\n#include <stdio.h>\n\nint main(void) {{\n",
        source.display()
    ));
    c.push_str(&c_body);
    c.push_str("    return 0;\n}\n");

    let mut rust = String::new();
    rust.push_str("fn main() {\n");
    rust.push_str(&r_body);
    rust.push_str("}\n");
    Ok(Drivers { c, rust })
}

/// The driver includes the C source, so the path must not depend on the
/// process working directory.
fn absolute(path: &Path) -> std::path::PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn call_driver(
    program: &Program,
    call: &Call,
    c_body: &mut String,
    r_body: &mut String,
) -> Result<(), Error> {
    let function = program
        .function_named(&call.function)
        .ok_or_else(|| Error::Case(format!("call target {} does not exist", call.function)))?;
    if function.linkage != Linkage::External {
        return Err(Error::Unsupported(format!(
            "{} is internal; the differential can only call exported functions",
            call.function
        )));
    }
    let arguments = arguments(program, function, &call.args)?;
    let c_call = format!("{}({})", function.name, arguments.c_args.join(", "));
    let r_call = format!(
        "generated::{}({})",
        function.name,
        arguments.r_args.join(", ")
    );
    let ret = function.signature.ret;
    // A typedef of a record returns a record: the canonical type decides how
    // the value is printed, and the spelling named by the typedef is the one
    // the source can compile. Every call runs in its own block, so a pointer
    // argument's place exists before the call and can be printed after it.
    let mut c_lines = arguments.c_setup;
    let mut r_lines = arguments.r_setup;
    let mut c_values = Vec::new();
    let mut r_values = Vec::new();

    match program.types.canonical(ret) {
        Some(Type::Void) => {
            c_lines.push(c_call);
            r_lines.push(r_call);
        }
        Some(Type::Record { .. }) => {
            let mut fields = Vec::new();
            flatten_fields(program, ret, "r", &mut fields)?;
            if fields.is_empty() {
                return Err(Error::Unsupported(
                    "a record with no printable field cannot be compared".to_string(),
                ));
            }
            let spelled = c_type(program, ret);
            c_lines.push(format!("{spelled} r = {c_call}"));
            r_lines.push(format!("let r = {r_call}"));
            for (path, _) in &fields {
                c_values.push(format!("(long long)({path})"));
                r_values.push(format!("{path} as i64"));
            }
        }
        Some(Type::Bool | Type::Char { .. } | Type::Int { .. } | Type::Enum { .. }) => {
            c_lines.push(format!("long long r = (long long)({c_call})"));
            r_lines.push(format!("let r = ({r_call}) as i64"));
            c_values.push("r".to_string());
            r_values.push("r".to_string());
        }
        Some(Type::Float { .. }) => {
            return Err(Error::Unsupported(
                "floating-point differential output is not part of this milestone".to_string(),
            ))
        }
        other => {
            return Err(Error::Unsupported(format!(
                "cannot print a return value of {other:?}"
            )))
        }
    }
    c_values.extend(arguments.c_observed);
    r_values.extend(arguments.r_observed);

    let c_print = if c_values.is_empty() {
        "printf(\"void\\n\");".to_string()
    } else {
        let format = vec!["%lld"; c_values.len()].join(" ");
        format!("printf(\"{format}\\n\", {});", c_values.join(", "))
    };
    let r_print = if r_values.is_empty() {
        "println!(\"void\");".to_string()
    } else {
        let format = vec!["{}"; r_values.len()].join(" ");
        format!("println!(\"{format}\", {});", r_values.join(", "))
    };

    c_body.push_str("    {\n");
    for line in &c_lines {
        c_body.push_str("        ");
        c_body.push_str(line);
        c_body.push_str(";\n");
    }
    c_body.push_str("        ");
    c_body.push_str(&c_print);
    c_body.push_str("\n    }\n");

    r_body.push_str("    {\n");
    for line in &r_lines {
        r_body.push_str("        ");
        r_body.push_str(line);
        r_body.push_str(";\n");
    }
    r_body.push_str("        ");
    r_body.push_str(&r_print);
    r_body.push_str("\n    }\n");
    Ok(())
}

/// One call's argument lists, plus the places a pointer argument needs.
pub(crate) struct Arguments {
    c_args: Vec<String>,
    r_args: Vec<String>,
    /// Statements that declare the place a pointer argument refers to.
    c_setup: Vec<String>,
    r_setup: Vec<String>,
    /// Expressions printed after the call, for a pointee the callee may write.
    c_observed: Vec<String>,
    r_observed: Vec<String>,
}

/// Build the two sides of one call's argument list. A pointer argument gets a
/// place of its own before the call, so an output parameter can be compared
/// after it.
pub(crate) fn arguments(
    program: &Program,
    function: &Function,
    values: &[Value],
) -> Result<Arguments, Error> {
    let params = &function.signature.params;
    let group = recast_ir::slice::slice_group(program, params);
    let mut arguments = Arguments {
        c_args: Vec::new(),
        r_args: Vec::new(),
        c_setup: Vec::new(),
        r_setup: Vec::new(),
        c_observed: Vec::new(),
        r_observed: Vec::new(),
    };
    let mut used = 0;
    let mut slice_len: Option<usize> = None;

    for (index, param) in params.iter().enumerate() {
        if group.as_ref().is_some_and(|group| group.length == index) {
            // The C parameter list keeps the length in its own position, so
            // the derived value goes here, not at the end of the call.
            let len = slice_len.ok_or_else(|| {
                Error::Case(format!(
                    "{} has a length parameter with no slice before it",
                    function.name
                ))
            })?;
            arguments.c_args.push(len.to_string());
            continue;
        }
        let value = values.get(used).ok_or_else(|| {
            Error::Case(format!(
                "{} expects more arguments than the case supplies",
                function.name
            ))
        })?;
        used += 1;
        if group
            .as_ref()
            .is_some_and(|group| group.contains_pointer(index))
        {
            let (c, rust, len) = slice_argument(program, param.ty, value)?;
            if let Some(existing) = slice_len {
                if existing != len {
                    return Err(Error::Case(format!(
                        "{} has two slices of different lengths",
                        function.name
                    )));
                }
            } else {
                slice_len = Some(len);
            }
            arguments.c_args.push(c);
            arguments.r_args.push(rust);
        } else if matches!(
            program.types.canonical(param.ty),
            Some(Type::Pointer { .. })
        ) {
            let pointer = pointer_argument(program, param.ty, value, index)?;
            arguments.c_args.push(pointer.c_arg);
            arguments.r_args.push(pointer.r_arg);
            arguments.c_setup.push(pointer.c_setup);
            arguments.r_setup.push(pointer.r_setup);
            if let (Some(c), Some(rust)) = (pointer.c_observed, pointer.r_observed) {
                arguments.c_observed.push(c);
                arguments.r_observed.push(rust);
            }
        } else {
            let (c, rust) = value_argument(program, param.ty, value)?;
            arguments.c_args.push(c);
            arguments.r_args.push(rust);
        }
    }
    if used != values.len() {
        return Err(Error::Case(format!(
            "{} received {used} arguments but the case supplies {}",
            function.name,
            values.len()
        )));
    }
    Ok(arguments)
}

struct PointerArgument {
    c_setup: String,
    r_setup: String,
    c_arg: String,
    r_arg: String,
    c_observed: Option<String>,
    r_observed: Option<String>,
}

/// A pointer to one scalar: the case value is the pointee's initial value. A
/// non-const pointer is an output parameter, so its value is printed after the
/// call as well.
fn pointer_argument(
    program: &Program,
    ty: TypeId,
    value: &Value,
    index: usize,
) -> Result<PointerArgument, Error> {
    let Some(Type::Pointer { pointee, is_const }) = program.types.canonical(ty) else {
        return Err(Error::Case("expected a pointer parameter".to_string()));
    };
    let rust_type = rust_scalar(program, *pointee).ok_or_else(|| {
        Error::Unsupported(
            "a pointer whose pointee is not a scalar is the next M2 step".to_string(),
        )
    })?;
    let c_type = c_type(program, *pointee);
    let (c_value, r_value) = value_argument(program, *pointee, value)?;
    let name = format!("a{index}");
    let mutable = !*is_const;
    Ok(PointerArgument {
        c_setup: format!("{c_type} {name} = {c_value}"),
        r_setup: if mutable {
            format!("let mut {name}: {rust_type} = {r_value}")
        } else {
            format!("let {name}: {rust_type} = {r_value}")
        },
        c_arg: format!("&{name}"),
        r_arg: if mutable {
            format!("&mut {name}")
        } else {
            format!("&{name}")
        },
        c_observed: mutable.then(|| format!("(long long)({name})")),
        r_observed: mutable.then(|| format!("{name} as i64")),
    })
}

/// The Rust spelling of a scalar the driver can own and print.
fn rust_scalar(program: &Program, id: TypeId) -> Option<String> {
    match program.types.canonical(id) {
        Some(Type::Bool) => Some("bool".to_string()),
        Some(Type::Char { signed }) => Some(if *signed { "i8" } else { "u8" }.to_string()),
        Some(Type::Int { bits, signed, .. }) => {
            Some(format!("{}{}", if *signed { 'i' } else { 'u' }, bits))
        }
        Some(Type::Enum { name }) => program
            .enum_named(name)
            .and_then(|item| rust_scalar(program, item.underlying)),
        _ => None,
    }
}

fn slice_argument(
    program: &Program,
    ty: TypeId,
    value: &Value,
) -> Result<(String, String, usize), Error> {
    let Some(Type::Pointer { pointee, .. }) = program.types.get(ty) else {
        return Err(Error::Case("expected a pointer parameter".to_string()));
    };
    let element = c_element_type(program, *pointee);
    match value {
        Value::String(text) if matches!(program.types.get(*pointee), Some(Type::Char { .. })) => {
            let signed = matches!(
                program.types.get(*pointee),
                Some(Type::Char { signed: true })
            );
            // A byte-string literal has type `&[u8; N]`. When the target has a
            // signed `char`, the translated parameter is `&[i8]`, so the
            // elements have to be spelled out instead of relying on the
            // literal's element type.
            let rust = if signed {
                let values = text
                    .as_bytes()
                    .iter()
                    .map(|byte| format!("{}i8", *byte as i8))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("&[{values}]")
            } else {
                format!("b\"{}\"", escape_bytes(text.as_bytes()))
            };
            Ok((format!("\"{}\"", escape_c(text)), rust, text.len()))
        }
        Value::Array(items) if items.is_empty() => {
            Ok((format!("({element} *)0"), "&[]".to_string(), 0))
        }
        Value::Array(items) => {
            let mut c_values = Vec::with_capacity(items.len());
            let mut rust_values = Vec::with_capacity(items.len());
            for item in items {
                let number = integer(item)?;
                c_values.push(number.to_string());
                rust_values.push(number.to_string());
            }
            Ok((
                format!("({element}[]){{{}}}", c_values.join(", ")),
                format!("&[{}]", rust_values.join(", ")),
                items.len(),
            ))
        }
        _ => Err(Error::Case(format!(
            "slice argument must be an array or string, found {value}"
        ))),
    }
}

fn value_argument(program: &Program, ty: TypeId, value: &Value) -> Result<(String, String), Error> {
    // The type as the source spells it: a typedef name for an alias, a
    // `struct`/`enum` tag otherwise. The driver includes the source, so both
    // are in scope.
    let spelled = c_type(program, ty);
    match program.types.canonical(ty) {
        Some(Type::Bool) => {
            let flag = value
                .as_bool()
                .ok_or_else(|| Error::Case(format!("expected a boolean, found {value}")))?;
            Ok((
                if flag { "1" } else { "0" }.to_string(),
                if flag { "true" } else { "false" }.to_string(),
            ))
        }
        Some(Type::Char { .. } | Type::Int { .. } | Type::Enum { .. }) => {
            let number = integer(value)?;
            Ok((number.to_string(), number.to_string()))
        }
        Some(Type::Record { name }) => {
            let object = value.as_object().ok_or_else(|| {
                Error::Case(format!("expected an object for {name}, found {value}"))
            })?;
            let record = program
                .record(name)
                .ok_or_else(|| Error::Case(format!("record {name} is not defined")))?;
            let mut c_fields = Vec::new();
            let mut r_fields = Vec::new();
            for field in &record.fields {
                let field_value = object.get(&field.name).ok_or_else(|| {
                    Error::Case(format!("record {name} is missing field {}", field.name))
                })?;
                let (c, rust) = value_argument(program, field.ty, field_value)?;
                c_fields.push(c);
                r_fields.push(format!("{}: {rust}", field.name));
            }
            Ok((
                format!("({spelled}){{{}}}", c_fields.join(", ")),
                format!("generated::{name} {{ {} }}", r_fields.join(", ")),
            ))
        }
        Some(Type::Array { elem, len }) => {
            let items = value
                .as_array()
                .ok_or_else(|| Error::Case(format!("expected an array, found {value}")))?;
            let expected = usize::try_from(*len).unwrap_or(usize::MAX);
            if items.len() != expected {
                return Err(Error::Case(format!(
                    "expected {expected} element(s), found {}",
                    items.len()
                )));
            }
            let mut c_values = Vec::with_capacity(items.len());
            let mut r_values = Vec::with_capacity(items.len());
            for item in items {
                let (c, rust) = value_argument(program, *elem, item)?;
                c_values.push(c);
                r_values.push(rust);
            }
            // An array member takes a braced initializer, not a compound
            // literal: `(struct w){{1, 2, 3}, 4}`. M1 has no array parameter
            // that would need the expression form.
            Ok((
                format!("{{{}}}", c_values.join(", ")),
                format!("[{}]", r_values.join(", ")),
            ))
        }
        Some(Type::Named { canonical, .. }) => value_argument(program, *canonical, value),
        other => Err(Error::Unsupported(format!(
            "cannot pass a value of type {other:?}"
        ))),
    }
}

fn flatten_fields(
    program: &Program,
    ty: TypeId,
    prefix: &str,
    out: &mut Vec<(String, TypeId)>,
) -> Result<(), Error> {
    match program.types.canonical(ty) {
        Some(Type::Record { name }) => {
            let record = program
                .record(name)
                .ok_or_else(|| Error::Case(format!("record {name} is not defined")))?;
            for field in &record.fields {
                flatten_fields(program, field.ty, &format!("{prefix}.{}", field.name), out)?;
            }
            Ok(())
        }
        Some(Type::Named { canonical, .. }) => flatten_fields(program, *canonical, prefix, out),
        Some(_) => {
            out.push((prefix.to_string(), ty));
            Ok(())
        }
        None => Err(Error::Case(format!("missing type {ty}"))),
    }
}

fn integer(value: &Value) -> Result<i64, Error> {
    value
        .as_i64()
        .ok_or_else(|| Error::Case(format!("expected an integer, found {value}")))
}

fn c_element_type(program: &Program, id: TypeId) -> String {
    match program.types.canonical(id) {
        Some(Type::Char { .. }) => "char".to_string(),
        Some(Type::Bool) => "_Bool".to_string(),
        // Use the source spelling (`int`, `unsigned int`, ...) so the driver
        // needs no `stdint.h` include of its own.
        Some(Type::Int { .. }) | Some(Type::Named { .. }) => c_type(program, id),
        _ => c_type(program, id),
    }
}

fn c_type(program: &Program, id: TypeId) -> String {
    match program.types.get(id) {
        Some(Type::Void) => "void".to_string(),
        Some(Type::Bool) => "_Bool".to_string(),
        Some(Type::Char { .. }) => "char".to_string(),
        Some(Type::Int { spelling, .. }) => spelling.clone(),
        Some(Type::Float { spelling, .. }) => spelling.clone(),
        Some(Type::Record { name }) => format!("struct {name}"),
        Some(Type::Enum { name }) => format!("enum {name}"),
        // A typedef name is the spelling the source uses, and the driver
        // includes the source, so `size_t`, `uint` and a record typedef are
        // all in scope. A tag needs its keyword: `struct Point`.
        Some(Type::Named { name, .. }) => name.clone(),
        Some(Type::Array { elem, len }) => format!("{}[{len}]", c_type(program, *elem)),
        _ => "long".to_string(),
    }
}

fn escape_c(text: &str) -> String {
    let mut out = String::new();
    for character in text.chars() {
        match character {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            other => out.push(other),
        }
    }
    out
}

fn escape_bytes(bytes: &[u8]) -> String {
    let mut out = String::new();
    for byte in bytes {
        match byte {
            b'\\' => out.push_str("\\\\"),
            b'"' => out.push_str("\\\""),
            b'\n' => out.push_str("\\n"),
            b'\t' => out.push_str("\\t"),
            0x20..=0x7e => out.push(*byte as char),
            other => out.push_str(&format!("\\x{other:02x}")),
        }
    }
    out
}

/// Build the property-sweep drivers for one function.
pub fn sweep(program: &Program, sweep: &Sweep, source: &Path) -> Result<Drivers, Error> {
    let function = program
        .function_named(&sweep.function)
        .ok_or_else(|| Error::Case(format!("sweep target {} does not exist", sweep.function)))?;
    let mut params = Vec::new();
    for param in &function.signature.params {
        let (bits, signed) = integer_shape(program, param.ty).ok_or_else(|| {
            Error::Unsupported(format!(
                "sweep parameters must be integers; {} is not",
                function.name
            ))
        })?;
        let name = param
            .name
            .clone()
            .ok_or_else(|| Error::Case("sweep parameter has no name".to_string()))?;
        params.push((name, bits, signed));
    }
    let (ret_bits, ret_signed) =
        integer_shape(program, function.signature.ret).ok_or_else(|| {
            Error::Unsupported(format!(
                "sweep return value must be an integer; {} is not",
                function.name
            ))
        })?;
    let _ = (ret_bits, ret_signed);

    let iterations = sweep.iterations;
    let seed = 0x243F_6A88_85A3_08D3u64;

    let mut c = String::new();
    c.push_str("#include <stdint.h>\n#include <stdio.h>\n");
    // Including the implementation keeps static helpers visible too.
    c.push_str(&format!("#include \"{}\"\n", absolute(source).display()));
    c.push_str(&format!(
        "\nstatic uint64_t next(uint64_t *state) {{\n    *state = *state * 6364136223846793005ULL + 1442695040888963407ULL;\n    return *state;\n}}\n\nint main(void) {{\n    uint64_t state = {seed}ULL;\n    uint64_t hash = 1469598103934665603ULL;\n    for (uint64_t i = 0; i < {iterations}ULL; i++) {{\n"
    ));
    for (name, bits, signed) in &params {
        let c_type = if *signed {
            format!("int{bits}_t")
        } else {
            format!("uint{bits}_t")
        };
        if *signed {
            // Keep signed operands small enough that `a + b` and `a * b`
            // stay inside the type: the sweep must not exercise C UB.
            let half = if *bits >= 10 {
                512u64
            } else {
                1u64 << (*bits - 2)
            };
            let mask = half * 2 - 1;
            c.push_str(&format!(
                "        {c_type} {name} = ({c_type})(((next(&state) >> 32) & {mask}ULL) - {half});\n"
            ));
        } else {
            c.push_str(&format!(
                "        {c_type} {name} = ({c_type})(next(&state) >> 32);\n"
            ));
        }
    }
    let names = params
        .iter()
        .map(|(name, _, _)| name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    c.push_str(&format!(
        "        long long result = (long long)({}({names}));\n        hash = hash * 1099511628211ULL + (uint64_t)result;\n    }}\n    printf(\"%llu\\n\", (unsigned long long)hash);\n    return 0;\n}}\n",
        function.name
    ));

    let mut rust = String::new();
    rust.push_str("fn main() {\n");
    rust.push_str(&format!("    let mut state: u64 = {seed}u64;\n"));
    rust.push_str("    let mut hash: u64 = 1469598103934665603u64;\n");
    rust.push_str(&format!("    for _ in 0..{iterations} {{\n"));
    for (name, bits, signed) in &params {
        // Each parameter advances the generator once, matching `next(&state)`.
        rust.push_str(
            "        state = state\n            .wrapping_mul(6364136223846793005)\n            .wrapping_add(1442695040888963407);\n",
        );
        if *signed {
            let half = if *bits >= 10 {
                512i64
            } else {
                1i64 << (*bits - 2)
            };
            let mask = (half * 2 - 1) as u64;
            rust.push_str(&format!(
                "        let {name}: i{bits} = (((state >> 32) & {mask}) as i64 - {half}) as i{bits};\n"
            ));
        } else {
            rust.push_str(&format!(
                "        let {name}: u{bits} = (state >> 32) as u{bits};\n"
            ));
        }
    }
    rust.push_str(&format!(
        "        let result = generated::{}({names}) as i64;\n        hash = hash.wrapping_mul(1099511628211).wrapping_add(result as u64);\n    }}\n    println!(\"{{}}\", hash);\n}}\n",
        function.name
    ));

    Ok(Drivers { c, rust })
}

fn integer_shape(program: &Program, id: TypeId) -> Option<(u16, bool)> {
    match program.types.canonical(id) {
        Some(Type::Int { bits, signed, .. }) => Some((u16::from(*bits), *signed)),
        Some(Type::Char { signed }) => Some((8, *signed)),
        // `uint1_t` does not exist; sweeps cover integer parameters only.
        Some(Type::Bool) => None,
        Some(Type::Enum { name }) => program
            .enum_named(name)
            .and_then(|item| integer_shape(program, item.underlying)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use recast_ir::Target;

    fn pointer_to_char(program: &mut Program, signed: bool) -> TypeId {
        let character = program.types.intern(Type::Char { signed });
        program.types.intern(Type::Pointer {
            pointee: character,
            is_const: true,
        })
    }

    #[test]
    fn array_arguments_are_built_from_their_elements() {
        let mut program = Program::new(Target::lp64("x86_64-unknown-linux-gnu"));
        let int = program.types.intern(Type::Int {
            bits: 32,
            signed: true,
            spelling: "int".to_string(),
        });
        let array = program.types.intern(Type::Array { elem: int, len: 3 });
        let (c, rust) =
            value_argument(&program, array, &serde_json::json!([1, 2, 3])).expect("array argument");
        assert_eq!(c, "{1, 2, 3}");
        assert_eq!(rust, "[1, 2, 3]");
        let wrong = value_argument(&program, array, &serde_json::json!([1, 2]));
        assert!(wrong.is_err(), "an argument must match the declared length");
    }

    #[test]
    fn string_arguments_follow_the_target_char_signedness() {
        let mut program = Program::new(Target::lp64("x86_64-unknown-linux-gnu"));

        let unsigned = pointer_to_char(&mut program, false);
        let (c, rust, len) = slice_argument(&program, unsigned, &Value::String("hi".to_string()))
            .expect("unsigned string argument");
        assert_eq!(c, "\"hi\"");
        assert_eq!(rust, "b\"hi\"");
        assert_eq!(len, 2);

        let signed = pointer_to_char(&mut program, true);
        let (_, rust, len) = slice_argument(&program, signed, &Value::String("hi".to_string()))
            .expect("signed string argument");
        assert_eq!(rust, "&[104i8, 105i8]");
        assert_eq!(len, 2);
    }
}
