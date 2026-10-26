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
    let (c_args, r_args) = arguments(program, function, &call.args)?;
    let c_call = format!("{}({})", function.name, c_args.join(", "));
    let r_call = format!("generated::{}({})", function.name, r_args.join(", "));
    let ret = function.signature.ret;
    match program.types.get(ret) {
        Some(Type::Void) => {
            c_body.push_str(&format!("    {c_call};\n    printf(\"void\\n\");\n"));
            r_body.push_str(&format!("    {r_call};\n    println!(\"void\");\n"));
        }
        Some(Type::Record { name }) => {
            let mut fields = Vec::new();
            flatten_fields(program, ret, "r", &mut fields)?;
            let c_type = format!("struct {name}");
            let format = vec!["%lld"; fields.len()].join(" ");
            let c_values = fields
                .iter()
                .map(|(path, _)| format!("(long long)({path})"))
                .collect::<Vec<_>>()
                .join(", ");
            c_body.push_str(&format!(
                "    {{ {c_type} r = {c_call}; printf(\"{format}\\n\", {c_values}); }}\n"
            ));
            let r_format = vec!["{}"; fields.len()].join(" ");
            let r_values = fields
                .iter()
                .map(|(path, _)| format!("{path} as i64"))
                .collect::<Vec<_>>()
                .join(", ");
            r_body.push_str(&format!(
                "    {{ let r = {r_call}; println!(\"{r_format}\", {r_values}); }}\n"
            ));
        }
        Some(Type::Bool | Type::Char { .. } | Type::Int { .. } | Type::Enum { .. })
        | Some(Type::Named { .. }) => {
            c_body.push_str(&format!(
                "    printf(\"%lld\\n\", (long long)({c_call}));\n"
            ));
            r_body.push_str(&format!("    println!(\"{{}}\", ({r_call}) as i64);\n"));
        }
        Some(Type::Float { .. }) => {
            return Err(Error::Unsupported(
                "floating-point differential output is not part of M1".to_string(),
            ))
        }
        other => {
            return Err(Error::Unsupported(format!(
                "cannot print a return value of {other:?}"
            )))
        }
    }
    Ok(())
}

/// Build the two sides of one call's argument list.
pub(crate) fn arguments(
    program: &Program,
    function: &Function,
    values: &[Value],
) -> Result<(Vec<String>, Vec<String>), Error> {
    let params = &function.signature.params;
    let length_index = slice_length_index(program, function);
    let mut c_args = Vec::new();
    let mut r_args = Vec::new();
    let mut used = 0;
    let mut slice_len: Option<usize> = None;

    for (index, param) in params.iter().enumerate() {
        if Some(index) == length_index {
            continue;
        }
        let value = values.get(used).ok_or_else(|| {
            Error::Case(format!(
                "{} expects more arguments than the case supplies",
                function.name
            ))
        })?;
        used += 1;
        if length_index.is_some_and(|length| index < length) {
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
            c_args.push(c);
            r_args.push(rust);
        } else {
            let (c, rust) = value_argument(program, param.ty, value)?;
            c_args.push(c);
            r_args.push(rust);
        }
    }
    if used != values.len() {
        return Err(Error::Case(format!(
            "{} received {used} arguments but the case supplies {}",
            function.name,
            values.len()
        )));
    }
    if let Some(len) = slice_len {
        c_args.push(len.to_string());
    }
    Ok((c_args, r_args))
}

fn slice_length_index(program: &Program, function: &Function) -> Option<usize> {
    let params = &function.signature.params;
    let index = params
        .iter()
        .position(|param| is_size_t(program, param.ty))?;
    if index == 0 {
        return None;
    }
    if params[..index]
        .iter()
        .all(|param| matches!(program.types.get(param.ty), Some(Type::Pointer { .. })))
    {
        Some(index)
    } else {
        None
    }
}

fn is_size_t(program: &Program, id: TypeId) -> bool {
    match program.types.get(id) {
        Some(Type::Named { name, .. }) if name == "size_t" => true,
        Some(Type::Int {
            bits,
            signed: false,
            ..
        }) => u16::from(*bits) == u16::from(program.target.size_t_bits),
        _ => false,
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
            Ok((
                format!("\"{}\"", escape_c(text)),
                format!("b\"{}\"", escape_bytes(text.as_bytes())),
                text.len(),
            ))
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
                format!("(struct {name}){{{}}}", c_fields.join(", ")),
                format!("generated::{name} {{ {} }}", r_fields.join(", ")),
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
        Some(Type::Named { name, canonical }) => {
            if name == "size_t" {
                "size_t".to_string()
            } else {
                c_type(program, *canonical)
            }
        }
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
