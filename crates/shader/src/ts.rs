//! TypeScript to JavaScript: Figma's shader `main.ts` with its types stripped.

use std::path::Path;

use oxc::allocator::Allocator;
use oxc::codegen::Codegen;
use oxc::parser::Parser;
use oxc::semantic::SemanticBuilder;
use oxc::span::SourceType;
use oxc::transformer::{TransformOptions, Transformer};

use crate::ShaderError;

/// The JavaScript of a TypeScript module (an ES module still: imports and exports stay).
pub(crate) fn to_js(ts: &str) -> Result<String, ShaderError> {
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, ts, SourceType::ts()).parse();
    if let Some(error) = parsed.diagnostics.errors().next() {
        return Err(ShaderError::script(format!(
            "main.ts does not parse: {error}"
        )));
    }
    if parsed.panicked {
        return Err(ShaderError::script("main.ts does not parse"));
    }
    let mut program = parsed.program;
    let scoping = SemanticBuilder::new()
        .build(&program)
        .semantic
        .into_scoping();
    let out = Transformer::new(
        &allocator,
        Path::new("main.ts"),
        &TransformOptions::default(),
    )
    .build_with_scoping(scoping, &mut program);
    if let Some(error) = out.diagnostics.errors().next() {
        return Err(ShaderError::script(format!(
            "main.ts does not compile: {error}"
        )));
    }
    Ok(Codegen::new().build(&program).code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn types_are_stripped_and_modules_kept() {
        let js = to_js("export function f(a: any, b: number): number { return a + b; }")
            .expect("compiles");
        assert!(js.contains("export function f(a, b)"), "{js}");
        assert!(to_js("export function (").is_err());
    }
}
