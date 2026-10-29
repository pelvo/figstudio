//! The shader's JavaScript in Boa: a `GPUDevice` whose calls go to [`Session`] (the wgpu
//! objects live there, the script holds `{__id}` handles), then `setup` and `render`.

use std::cell::RefCell;
use std::rc::Rc;

use boa_engine::builtins::promise::PromiseState;
use boa_engine::module::MapModuleLoader;
use boa_engine::native_function::NativeFunctionPointer;
use boa_engine::object::ObjectInitializer;
use boa_engine::object::builtins::{JsArray, JsArrayBuffer, JsTypedArray};
use boa_engine::property::Attribute;
use boa_engine::{
    Context, JsArgs, JsError, JsNativeError, JsObject, JsResult, JsString, JsValue, Module,
    NativeFunction, Source, js_string,
};
use serde_json::{Value, json};

use crate::ShaderError;
use crate::gpu::{self, Object, Op, Pass, Session};

thread_local! {
    static SESSION: RefCell<Option<Session>> = const { RefCell::new(None) };
}

/// Most loop iterations a script may run (a shader's setup and render are short).
const LOOP_LIMIT: u64 = 10_000_000;
/// Deepest call recursion a script may use.
const RECURSION_LIMIT: usize = 512;

/// WebGPU's flag enums, with the values of the WebGPU spec (the same bits wgpu uses).
const GLOBALS: &str = "
globalThis.GPUBufferUsage = { MAP_READ: 1, MAP_WRITE: 2, COPY_SRC: 4, COPY_DST: 8, INDEX: 16,
  VERTEX: 32, UNIFORM: 64, STORAGE: 128, INDIRECT: 256, QUERY_RESOLVE: 512 };
globalThis.GPUTextureUsage = { COPY_SRC: 1, COPY_DST: 2, TEXTURE_BINDING: 4,
  STORAGE_BINDING: 8, RENDER_ATTACHMENT: 16 };
globalThis.GPUShaderStage = { VERTEX: 1, FRAGMENT: 2, COMPUTE: 4 };
globalThis.GPUMapMode = { READ: 1, WRITE: 2 };
globalThis.GPUColorWrite = { RED: 1, GREEN: 2, BLUE: 4, ALPHA: 8, ALL: 15 };
";

/// Clears the thread's session when a run ends, however it ends.
struct Clear;

impl Drop for Clear {
    fn drop(&mut self) {
        SESSION.with(|s| s.borrow_mut().take());
    }
}

/// Runs `js` (the shader module) on the GPU into a `width × height` output and returns its
/// straight-alpha RGBA8 pixels.
pub(crate) fn run(
    js: &str,
    params: &Value,
    width: u16,
    height: u16,
    time_ms: f64,
) -> Result<Vec<u8>, ShaderError> {
    let (device, queue) = gpu::device().ok_or_else(ShaderError::no_gpu)?;
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    SESSION.with(|s| {
        *s.borrow_mut() = Some(Session {
            device: device.clone(),
            queue: queue.clone(),
            objects: Vec::new(),
        });
    });
    let clear = Clear;
    let pixels = script(js, params, width, height, time_ms);
    drop(clear);
    if let Some(error) = pollster::block_on(scope.pop()) {
        return Err(ShaderError::gpu(error.to_string()));
    }
    pixels
}

fn script(
    js: &str,
    params: &Value,
    width: u16,
    height: u16,
    time_ms: f64,
) -> Result<Vec<u8>, ShaderError> {
    let loader = Rc::new(MapModuleLoader::new());
    let mut ctx = Context::builder()
        .module_loader(loader.clone())
        .build()
        .map_err(script_error)?;
    ctx.runtime_limits_mut()
        .set_loop_iteration_limit(LOOP_LIMIT);
    ctx.runtime_limits_mut()
        .set_recursion_limit(RECURSION_LIMIT);
    let figma = Module::parse(
        Source::from_bytes(b"export function defineProperties() {}"),
        None,
        &mut ctx,
    )
    .map_err(script_error)?;
    loader.insert("figma:shaders", figma);
    ctx.eval(Source::from_bytes(GLOBALS))
        .map_err(script_error)?;

    let module =
        Module::parse(Source::from_bytes(js.as_bytes()), None, &mut ctx).map_err(script_error)?;
    let loaded = module.load_link_evaluate(&mut ctx);
    ctx.run_jobs().map_err(script_error)?;
    match loaded.state() {
        PromiseState::Fulfilled(_) => {}
        PromiseState::Rejected(e) => {
            return Err(script_error(JsError::from_opaque(e)));
        }
        PromiseState::Pending => return Err(ShaderError::script("main.ts did not finish loading")),
    }
    let namespace = module.namespace(&mut ctx);
    let entry = |name: JsString, ctx: &mut Context| -> Result<Option<JsObject>, ShaderError> {
        let value = namespace.get(name, ctx).map_err(script_error)?;
        Ok(value.as_callable())
    };
    let setup = entry(js_string!("setup"), &mut ctx)?;
    let render = entry(js_string!("render"), &mut ctx)?
        .ok_or_else(|| ShaderError::script("main.ts exports no render function"))?;

    let output = with_session(|s| {
        s.create_texture(&json!({
            "size": [width, height],
            "format": "rgba8unorm",
            // RENDER_ATTACHMENT | TEXTURE_BINDING | COPY_SRC
            "usage": 16 | 4 | 1,
        }))
    })?;
    let device = device_object(&mut ctx);
    let frame = JsValue::from_json(
        &json!({
            "params": params,
            "state": {},
            "input": null,
            "time": time_ms,
            "deltaTime": 0,
            "frame": 0,
            "mousePosition": { "x": 0, "y": 0 },
        }),
        &mut ctx,
    )
    .map_err(script_error)?;
    let output_object = texture_object(&mut ctx, output, width, height);
    if let Some(frame) = frame.as_object() {
        frame
            .set(js_string!("output"), output_object, true, &mut ctx)
            .map_err(script_error)?;
    }
    let args = [device.into(), frame];
    if let Some(setup) = setup {
        setup
            .call(&JsValue::undefined(), &args, &mut ctx)
            .map_err(script_error)?;
    }
    render
        .call(&JsValue::undefined(), &args, &mut ctx)
        .map_err(script_error)?;
    ctx.run_jobs().map_err(script_error)?;
    with_session(|s| s.read(output))
}

fn script_error(e: JsError) -> ShaderError {
    ShaderError::script(format!("main.ts: {e}"))
}

fn with_session<R>(
    f: impl FnOnce(&mut Session) -> Result<R, ShaderError>,
) -> Result<R, ShaderError> {
    SESSION.with(|s| {
        let mut s = s.borrow_mut();
        let s = s
            .as_mut()
            .ok_or_else(|| ShaderError::script("no GPU session"))?;
        f(s)
    })
}

/// [`with_session`] for a native function: errors become JavaScript `TypeError`s.
fn with<R>(f: impl FnOnce(&mut Session) -> Result<R, ShaderError>) -> JsResult<R> {
    with_session(f).map_err(|e| JsNativeError::typ().with_message(e.to_string()).into())
}

fn arg(args: &[JsValue], i: usize, ctx: &mut Context) -> JsResult<Value> {
    Ok(args
        .get_or_undefined(i)
        .to_json(ctx)?
        .unwrap_or(Value::Null))
}

fn number(args: &[JsValue], i: usize, ctx: &mut Context) -> JsResult<f64> {
    let v = args.get_or_undefined(i);
    if v.is_undefined() {
        Ok(0.0)
    } else {
        v.to_number(ctx)
    }
}

/// The object index of a handle's own `__id`.
fn id_of(value: &JsValue, ctx: &mut Context) -> JsResult<usize> {
    let object = value
        .as_object()
        .ok_or_else(|| JsNativeError::typ().with_message("expected a GPU object"))?;
    Ok(object.get(js_string!("__id"), ctx)?.to_u32(ctx)? as usize)
}

type Methods = [(&'static str, NativeFunctionPointer, usize)];

/// A handle object for object `id`, with `methods`.
fn handle(ctx: &mut Context, id: usize, methods: &Methods) -> JsObject {
    let mut init = ObjectInitializer::new(ctx);
    init.property(js_string!("__id"), id as u32, Attribute::all());
    for &(name, f, length) in methods {
        init.function(NativeFunction::from_fn_ptr(f), JsString::from(name), length);
    }
    init.build()
}

fn device_object(ctx: &mut Context) -> JsObject {
    let queue = ObjectInitializer::new(ctx)
        .function(
            NativeFunction::from_fn_ptr(write_buffer),
            js_string!("writeBuffer"),
            3,
        )
        .function(NativeFunction::from_fn_ptr(submit), js_string!("submit"), 1)
        .build();
    let mut init = ObjectInitializer::new(ctx);
    init.property(js_string!("queue"), queue, Attribute::all());
    let methods: &Methods = &[
        ("createShaderModule", create_shader_module, 1),
        ("createBuffer", create_buffer, 1),
        ("createTexture", create_texture, 1),
        ("createSampler", create_sampler, 1),
        ("createRenderPipeline", create_render_pipeline, 1),
        ("createBindGroup", create_bind_group, 1),
        ("createCommandEncoder", create_command_encoder, 0),
    ];
    for &(name, f, length) in methods {
        init.function(NativeFunction::from_fn_ptr(f), JsString::from(name), length);
    }
    init.build()
}

fn create_shader_module(_: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let desc = arg(args, 0, ctx)?;
    let code = desc["code"].as_str().unwrap_or_default().to_owned();
    let id = with(|s| {
        let module = s.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: None,
            source: wgpu::ShaderSource::Wgsl(code.into()),
        });
        Ok(s.add(Object::Module(module)))
    })?;
    Ok(handle(ctx, id, &[]).into())
}

fn create_buffer(_: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let desc = arg(args, 0, ctx)?;
    let size = (gpu::num(desc.get("size")) as u64).next_multiple_of(4);
    let usage = gpu::num(desc.get("usage")) as u32;
    let id = with(|s| {
        let buffer = s.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size,
            // Written with `queue.writeBuffer` when unmapped after `mappedAtCreation`.
            usage: wgpu::BufferUsages::from_bits_truncate(usage) | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Ok(s.add(Object::Buffer(buffer)))
    })?;
    let object = handle(
        ctx,
        id,
        &[
            ("getMappedRange", get_mapped_range, 0),
            ("unmap", unmap, 0),
            ("destroy", nothing, 0),
        ],
    );
    object.set(js_string!("size"), size as f64, true, ctx)?;
    object.set(js_string!("usage"), usage, true, ctx)?;
    if desc["mappedAtCreation"].as_bool() == Some(true) {
        let mapped = JsArrayBuffer::new(size as usize, ctx)?;
        object.set(js_string!("__mapped"), mapped, true, ctx)?;
    }
    Ok(object.into())
}

/// The whole mapped range; a buffer here is only mapped at creation.
fn get_mapped_range(this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    if number(args, 0, ctx)? != 0.0 {
        return Err(JsNativeError::typ()
            .with_message("getMappedRange with an offset is not supported")
            .into());
    }
    let object = this
        .as_object()
        .ok_or_else(|| JsNativeError::typ().with_message("not a GPUBuffer"))?;
    let mapped = object.get(js_string!("__mapped"), ctx)?;
    if mapped.is_undefined() {
        return Err(JsNativeError::typ()
            .with_message("the buffer is not mapped")
            .into());
    }
    Ok(mapped)
}

fn unmap(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = id_of(this, ctx)?;
    let Some(object) = this.as_object() else {
        return Ok(JsValue::undefined());
    };
    let mapped = object.get(js_string!("__mapped"), ctx)?;
    if let Some(buffer) = mapped.as_object() {
        let bytes = JsArrayBuffer::from_object(buffer.clone())?
            .data()
            .map(|d| d.to_vec())
            .unwrap_or_default();
        with(|s| {
            s.queue.write_buffer(s.buffer(id)?, 0, &bytes);
            Ok(())
        })?;
        object.set(js_string!("__mapped"), JsValue::undefined(), true, ctx)?;
    }
    Ok(JsValue::undefined())
}

fn nothing(_: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    Ok(JsValue::undefined())
}

fn texture_object(ctx: &mut Context, id: usize, width: u16, height: u16) -> JsObject {
    let mut init = ObjectInitializer::new(ctx);
    init.property(js_string!("__id"), id as u32, Attribute::all())
        .property(js_string!("width"), width, Attribute::all())
        .property(js_string!("height"), height, Attribute::all())
        .property(
            js_string!("format"),
            js_string!("rgba8unorm"),
            Attribute::all(),
        )
        .function(
            NativeFunction::from_fn_ptr(create_view),
            js_string!("createView"),
            0,
        )
        .function(
            NativeFunction::from_fn_ptr(nothing),
            js_string!("destroy"),
            0,
        );
    init.build()
}

fn create_texture(_: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let desc = arg(args, 0, ctx)?;
    let id = with(|s| s.create_texture(&desc))?;
    let (w, h) = with(|s| {
        let t = s.texture(id)?;
        Ok((t.width(), t.height()))
    })?;
    let object = texture_object(
        ctx,
        id,
        u16::try_from(w).unwrap_or(u16::MAX),
        u16::try_from(h).unwrap_or(u16::MAX),
    );
    let format = desc["format"].as_str().unwrap_or("rgba8unorm");
    object.set(js_string!("format"), JsString::from(format), true, ctx)?;
    Ok(object.into())
}

fn create_view(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let texture = id_of(this, ctx)?;
    let id = with(|s| {
        let view = s.texture(texture)?.create_view(&Default::default());
        Ok(s.add(Object::View(view)))
    })?;
    Ok(handle(ctx, id, &[]).into())
}

fn create_sampler(_: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let desc = arg(args, 0, ctx)?;
    let id = with(|s| Ok(s.create_sampler(&desc)))?;
    Ok(handle(ctx, id, &[]).into())
}

fn create_render_pipeline(_: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let desc = arg(args, 0, ctx)?;
    let id = with(|s| s.create_pipeline(&desc))?;
    Ok(handle(ctx, id, &[("getBindGroupLayout", get_bind_group_layout, 1)]).into())
}

fn get_bind_group_layout(this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let pipeline = id_of(this, ctx)?;
    let index = number(args, 0, ctx)? as u32;
    let id = with(|s| {
        let layout = s.pipeline(pipeline)?.get_bind_group_layout(index);
        Ok(s.add(Object::Layout(layout)))
    })?;
    Ok(handle(ctx, id, &[]).into())
}

fn create_bind_group(_: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let desc = arg(args, 0, ctx)?;
    let id = with(|s| s.create_bind_group(&desc))?;
    Ok(handle(ctx, id, &[]).into())
}

fn create_command_encoder(_: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = with(|s| Ok(s.add(Object::Encoder(Vec::new()))))?;
    Ok(handle(
        ctx,
        id,
        &[
            ("beginRenderPass", begin_render_pass, 1),
            ("finish", finish, 0),
        ],
    )
    .into())
}

fn finish(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = id_of(this, ctx)?;
    Ok(handle(ctx, id, &[]).into())
}

fn begin_render_pass(this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let encoder = id_of(this, ctx)?;
    let desc = arg(args, 0, ctx)?;
    let mut targets = Vec::new();
    for a in desc["colorAttachments"]
        .as_array()
        .map_or(&[][..], Vec::as_slice)
    {
        let view = gpu::handle(&a["view"])
            .map_err(|e| JsNativeError::typ().with_message(e.to_string()))?;
        let clear = (a["loadOp"].as_str() == Some("clear")).then(|| {
            let c = &a["clearValue"];
            let channel = |k: &str, i: usize| c.get(k).or_else(|| c.get(i)).and_then(Value::as_f64);
            wgpu::Color {
                r: channel("r", 0).unwrap_or(0.0),
                g: channel("g", 1).unwrap_or(0.0),
                b: channel("b", 2).unwrap_or(0.0),
                a: channel("a", 3).unwrap_or(0.0),
            }
        });
        targets.push((view, clear, a["storeOp"].as_str() != Some("discard")));
    }
    let index = with(|s| match s.objects.get_mut(encoder) {
        Some(Object::Encoder(passes)) => {
            passes.push(Pass {
                targets,
                ops: Vec::new(),
            });
            Ok(passes.len() - 1)
        }
        _ => Err(ShaderError::script("not a GPUCommandEncoder")),
    })?;
    let pass = handle(
        ctx,
        encoder,
        &[
            ("setPipeline", set_pipeline, 1),
            ("setBindGroup", set_bind_group, 2),
            ("setVertexBuffer", set_vertex_buffer, 2),
            ("setIndexBuffer", set_index_buffer, 2),
            ("draw", draw, 1),
            ("drawIndexed", draw_indexed, 1),
            ("end", nothing, 0),
        ],
    );
    pass.set(js_string!("__pass"), index as u32, true, ctx)?;
    Ok(pass.into())
}

/// Records `op` in the pass `this`.
fn record(this: &JsValue, ctx: &mut Context, op: Op) -> JsResult<JsValue> {
    let encoder = id_of(this, ctx)?;
    let index = this
        .as_object()
        .map(|o| o.get(js_string!("__pass"), ctx))
        .transpose()?
        .map(|v| v.to_u32(ctx))
        .transpose()?
        .unwrap_or(0) as usize;
    with(|s| match s.objects.get_mut(encoder) {
        Some(Object::Encoder(passes)) => {
            passes
                .get_mut(index)
                .ok_or_else(|| ShaderError::script("unknown render pass"))?
                .ops
                .push(op);
            Ok(())
        }
        _ => Err(ShaderError::script("not a GPURenderPassEncoder")),
    })?;
    Ok(JsValue::undefined())
}

fn set_pipeline(this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = id_of(args.get_or_undefined(0), ctx)?;
    record(this, ctx, Op::Pipeline(id))
}

fn set_bind_group(this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let index = number(args, 0, ctx)? as u32;
    let id = id_of(args.get_or_undefined(1), ctx)?;
    record(this, ctx, Op::BindGroup(index, id))
}

fn set_vertex_buffer(this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let slot = number(args, 0, ctx)? as u32;
    let id = id_of(args.get_or_undefined(1), ctx)?;
    let offset = number(args, 2, ctx)? as u64;
    record(this, ctx, Op::Vertex(slot, id, offset))
}

fn set_index_buffer(this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = id_of(args.get_or_undefined(0), ctx)?;
    let format = match args
        .get_or_undefined(1)
        .to_string(ctx)?
        .to_std_string_escaped()
        .as_str()
    {
        "uint16" => wgpu::IndexFormat::Uint16,
        _ => wgpu::IndexFormat::Uint32,
    };
    let offset = number(args, 2, ctx)? as u64;
    record(this, ctx, Op::Index(id, format, offset))
}

/// Argument `i` as a count, `default` when left out.
fn count(args: &[JsValue], i: usize, default: u32, ctx: &mut Context) -> JsResult<u32> {
    if args.get_or_undefined(i).is_undefined() {
        Ok(default)
    } else {
        Ok(args[i].to_u32(ctx)?)
    }
}

fn draw(this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let op = Op::Draw([
        count(args, 0, 0, ctx)?,
        count(args, 1, 1, ctx)?,
        count(args, 2, 0, ctx)?,
        count(args, 3, 0, ctx)?,
    ]);
    record(this, ctx, op)
}

fn draw_indexed(this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let op = Op::DrawIndexed(
        [
            count(args, 0, 0, ctx)?,
            count(args, 1, 1, ctx)?,
            count(args, 2, 0, ctx)?,
        ],
        args.get_or_undefined(3).to_i32(ctx)?,
        count(args, 4, 0, ctx)?,
    );
    record(this, ctx, op)
}

/// `queue.writeBuffer(buffer, offset, data, dataOffset?, size?)`: offsets in elements for a
/// typed array, in bytes for an `ArrayBuffer`.
fn write_buffer(_: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = id_of(args.get_or_undefined(0), ctx)?;
    let offset = number(args, 1, ctx)? as u64;
    let data = args
        .get_or_undefined(2)
        .as_object()
        .ok_or_else(|| JsNativeError::typ().with_message("writeBuffer needs data"))?;
    let (bytes, element) = if let Ok(array) = JsTypedArray::from_object(data.clone()) {
        let start = array.byte_offset(ctx)?;
        let length = array.byte_length(ctx)?;
        let elements = array.length(ctx)?.max(1);
        let buffer = array.buffer(ctx)?;
        let buffer = buffer
            .as_object()
            .ok_or_else(|| JsNativeError::typ().with_message("typed array without a buffer"))?;
        let all = JsArrayBuffer::from_object(buffer.clone())?
            .data()
            .map(|d| d.to_vec())
            .unwrap_or_default();
        let bytes = all.get(start..start + length).unwrap_or_default().to_vec();
        (bytes, length / elements)
    } else {
        let bytes = JsArrayBuffer::from_object(data.clone())?
            .data()
            .map(|d| d.to_vec())
            .unwrap_or_default();
        (bytes, 1)
    };
    let from = number(args, 3, ctx)? as usize * element;
    let size = if args.get_or_undefined(4).is_undefined() {
        bytes.len().saturating_sub(from)
    } else {
        number(args, 4, ctx)? as usize * element
    };
    let slice = bytes
        .get(from..from + size)
        .ok_or_else(|| JsNativeError::range().with_message("writeBuffer range past the data"))?;
    with(|s| {
        s.queue.write_buffer(s.buffer(id)?, offset, slice);
        Ok(())
    })?;
    Ok(JsValue::undefined())
}

fn submit(_: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let list = args
        .get_or_undefined(0)
        .as_object()
        .ok_or_else(|| JsNativeError::typ().with_message("submit needs an array"))?;
    let list = JsArray::from_object(list.clone())?;
    let mut ids = Vec::new();
    for i in 0..list.length(ctx)? {
        ids.push(id_of(&list.get(i, ctx)?, ctx)?);
    }
    with(|s| s.submit(&ids))?;
    Ok(JsValue::undefined())
}
