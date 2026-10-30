//! TypeScript to JavaScript: Figma's shader `main.ts` with its types stripped.

use std::path::Path;

use oxc::allocator::Allocator;
use oxc::ast::ast::{
    ArrowFunctionBody, ArrowFunctionExpression, DoWhileStatement, ForInStatement, ForOfStatement,
    ForStatement, Function, Statement, WhileStatement,
};
use oxc::ast_visit::{Visit, walk};
use oxc::codegen::Codegen;
use oxc::parser::Parser;
use oxc::semantic::SemanticBuilder;
use oxc::span::{GetSpan, SourceType};
use oxc::syntax::scope::ScopeFlags;
use oxc::transformer::{TransformOptions, Transformer};

use crate::ShaderError;

/// The JavaScript of a TypeScript module (an ES module still: imports and exports stay).
#[cfg_attr(
    not(feature = "native"),
    expect(dead_code, reason = "Boa runs it natively")
)]
pub(crate) fn to_js(ts: &str) -> Result<String, ShaderError> {
    compile(ts, false)
}

/// Loop iterations and function calls one browser run may take (the busiest playground shader
/// takes about 2 million; a page's JIT runs 20 million in well under a second).
const BROWSER_STEPS: u64 = 20_000_000;

/// [`to_js`] without the `figma:shaders` import: a local `defineProperties` stands in, so the
/// module imports nothing (a browser runs it from a blob URL).
///
/// Every loop body and function body first calls a step counter that throws past
/// [`BROWSER_STEPS`], so a shader that never returns stops even where its sandboxed frame shares
/// the page's thread (WebKit), and a `catch` cannot keep it going: the next iteration or call
/// throws again. The module exports `__fsr_<tag>` to reset the counter before each run; the
/// tag is a hash of the source, so the code itself cannot name the counter or its reset.
pub(crate) fn to_browser_js(ts: &str) -> Result<String, ShaderError> {
    to_guarded_js(ts, BROWSER_STEPS)
}

fn to_guarded_js(ts: &str, steps: u64) -> Result<String, ShaderError> {
    use std::hash::{BuildHasher, BuildHasherDefault, DefaultHasher};
    let tag = format!(
        "{:016x}",
        BuildHasherDefault::<DefaultHasher>::default().hash_one(ts)
    );
    let guarded = guard(ts, &format!("__fsg_{tag}()"))?;
    Ok(format!(
        "function defineProperties() {{}}\n\
         let __fsn_{tag} = 0;\n\
         const __fsg_{tag} = () => {{ if (++__fsn_{tag} > {steps}) \
         throw new RangeError(\"the shader ran past its step limit\"); }};\n\
         export function __fsr_{tag}() {{ __fsn_{tag} = 0; }}\n{}",
        compile(&guarded, true)?
    ))
}

/// `ts` with `call` (an expression) at the start of every loop body and function body.
fn guard(ts: &str, call: &str) -> Result<String, ShaderError> {
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
    let mut points = Points {
        call,
        at: Vec::new(),
    };
    points.visit_program(&parsed.program);
    let mut at = points.at;
    // From the end, so earlier offsets stay valid; equal offsets keep their order reversed,
    // which only ever puts closing brackets side by side.
    at.sort_by_key(|(offset, _)| std::cmp::Reverse(*offset));
    let mut out = ts.to_owned();
    for (offset, text) in at {
        out.insert_str(offset as usize, &text);
    }
    Ok(out)
}

/// Where [`guard`] inserts its calls (byte offsets into the source).
struct Points<'c> {
    call: &'c str,
    at: Vec<(u32, String)>,
}

impl Points<'_> {
    /// At the start of a loop's body, wrapping a single statement in a block.
    fn body(&mut self, body: &Statement<'_>) {
        if let Statement::BlockStatement(block) = body {
            self.at
                .push((block.span.start + 1, format!("{};", self.call)));
        } else {
            let span = body.span();
            self.at.push((span.start, format!("{{{};", self.call)));
            self.at.push((span.end, "}".to_owned()));
        }
    }
}

impl<'a> Visit<'a> for Points<'_> {
    fn visit_function(&mut self, it: &Function<'a>, flags: ScopeFlags) {
        if let Some(body) = &it.body {
            self.at
                .push((body.span.start + 1, format!("{};", self.call)));
        }
        walk::walk_function(self, it, flags);
    }

    fn visit_arrow_function_expression(&mut self, it: &ArrowFunctionExpression<'a>) {
        match &it.body {
            ArrowFunctionBody::FunctionBody(body) => {
                self.at
                    .push((body.span.start + 1, format!("{};", self.call)));
            }
            // `x => expr` becomes `x => (call, expr)`.
            expression => {
                let span = expression.span();
                self.at.push((span.start, format!("({}, ", self.call)));
                self.at.push((span.end, ")".to_owned()));
            }
        }
        walk::walk_arrow_function_expression(self, it);
    }

    fn visit_for_statement(&mut self, it: &ForStatement<'a>) {
        self.body(&it.body);
        walk::walk_for_statement(self, it);
    }

    fn visit_for_in_statement(&mut self, it: &ForInStatement<'a>) {
        self.body(&it.body);
        walk::walk_for_in_statement(self, it);
    }

    fn visit_for_of_statement(&mut self, it: &ForOfStatement<'a>) {
        self.body(&it.body);
        walk::walk_for_of_statement(self, it);
    }

    fn visit_while_statement(&mut self, it: &WhileStatement<'a>) {
        self.body(&it.body);
        walk::walk_while_statement(self, it);
    }

    fn visit_do_while_statement(&mut self, it: &DoWhileStatement<'a>) {
        self.body(&it.body);
        walk::walk_do_while_statement(self, it);
    }
}

fn compile(ts: &str, drop_figma_import: bool) -> Result<String, ShaderError> {
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
    if drop_figma_import {
        program.body.retain(|statement| {
            !matches!(statement, oxc::ast::ast::Statement::ImportDeclaration(import)
                if import.source.value == "figma:shaders")
        });
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

    /// `js` parses as a JavaScript module.
    fn parses(js: &str) -> bool {
        let allocator = Allocator::default();
        let parsed = Parser::new(&allocator, js, SourceType::mjs()).parse();
        parsed.diagnostics.errors().next().is_none() && !parsed.panicked
    }

    #[test]
    fn every_loop_and_function_calls_the_step_counter() {
        let js = to_browser_js(
            "export function render(d: any) {\n\
             for (let i = 0; i < 3; i++) x();\n\
             while (a) { b(); }\n\
             do c(); while (d);\n\
             for (const k in o) for (const v of o[k]) e(v);\n\
             const f = (n: number) => n * 2;\n\
             const g = () => { return 1; };\n\
             class C { m() { return 2; } }\n\
             }",
        )
        .unwrap();
        assert!(parses(&js), "{js}");
        // The counter's definition names it once; then 9 bodies call it: render, 5 loops, two
        // arrows and a method.
        assert_eq!(js.matches("__fsg_").count(), 1 + 9, "{js}");
    }

    #[test]
    fn the_playground_shaders_still_parse_once_guarded() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/shader-sources");
        let mut seen = 0;
        for entry in std::fs::read_dir(dir).unwrap() {
            let main = entry.unwrap().path().join("main.ts");
            let js = to_browser_js(&std::fs::read_to_string(&main).unwrap()).unwrap();
            assert!(parses(&js), "{}", main.display());
            seen += 1;
        }
        assert_eq!(seen, 12);
    }

    /// Runs the guarded module of `ts` (counter limit `steps`) in Boa: calls its reset, then
    /// `render`, and returns the error message, if any.
    #[cfg(feature = "native")]
    fn run_guarded(ts: &str, steps: u64) -> Result<(), String> {
        use boa_engine::builtins::promise::PromiseState;
        use boa_engine::{Context, Module, Source, js_string};
        let js = to_guarded_js(ts, steps).unwrap();
        let mut ctx = Context::builder()
            .instructions_remaining(usize::MAX)
            .build()
            .unwrap();
        // If the counter failed, Boa's own loop limit would stop the test with another message.
        ctx.runtime_limits_mut()
            .set_loop_iteration_limit(10_000_000);
        let module = Module::parse(Source::from_bytes(js.as_bytes()), None, &mut ctx).unwrap();
        let loaded = module.load_link_evaluate(&mut ctx);
        ctx.run_jobs().unwrap();
        assert!(matches!(loaded.state(), PromiseState::Fulfilled(_)));
        let namespace = module.namespace(&mut ctx);
        for key in namespace.own_property_keys(&mut ctx).unwrap() {
            let name = key.to_string();
            if name.starts_with("__fsr_") {
                let reset = namespace.get(key, &mut ctx).unwrap();
                reset
                    .as_callable()
                    .unwrap()
                    .call(&boa_engine::JsValue::undefined(), &[], &mut ctx)
                    .unwrap();
            }
        }
        let render = namespace.get(js_string!("render"), &mut ctx).unwrap();
        render
            .as_callable()
            .unwrap()
            .call(&boa_engine::JsValue::undefined(), &[], &mut ctx)
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    #[test]
    #[cfg(feature = "native")]
    fn endless_code_stops_on_the_step_counter_even_when_it_catches() {
        let error = run_guarded(
            "export function render() { for (;;) { try { for (;;) {} } catch (e) {} } }",
            1_000,
        )
        .unwrap_err();
        assert!(error.contains("step limit"), "{error}");
        let error = run_guarded(
            "const f = (n: number): number => { try { return f(n + 1); } catch (e) { return f(n); } };\n\
             export function render() { f(0); }",
            // Under Boa's own recursion limit, so the counter is what stops it.
            100,
        )
        .unwrap_err();
        assert!(error.contains("step limit"), "{error}");
        // Normal code within the limit runs.
        run_guarded(
            "export function render() { let s = 0; for (let i = 0; i < 100; i++) { s += i; } }",
            1_000,
        )
        .unwrap();
    }

    #[test]
    fn the_browser_module_imports_nothing() {
        let js = to_browser_js(
            "import { defineProperties } from \"figma:shaders\";\nexport default function Effect() {}\ndefineProperties(Effect, {});",
        )
        .unwrap();
        assert!(!js.contains("import"), "{js}");
        assert!(js.starts_with("function defineProperties() {}"), "{js}");
        assert!(js.contains("defineProperties(Effect, {})"), "{js}");
    }
}
