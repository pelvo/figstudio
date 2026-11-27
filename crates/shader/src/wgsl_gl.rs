//! OpenGL/GLES combined-sampler compatibility on `wgpu::Backend::Gl`.
//!
//! `naga::back::glsl` rejects any entry point that samples one texture with multiple samplers
//! (`Error::ImageMultipleSamplers`), and `wgpu-hal::gles` maps each pipeline texture slot to at
//! most one sampler (`sampler_map: [Option<u8>; MAX_TEXTURE_SLOTS]`), so cross-stage different
//! samplers on the same texture binding silently overwrite each other.
//!
//! Ordinary pipelines stay on stock `ShaderSource::Wgsl` with zero Naga work. On `Backend::Gl`,
//! [`lower_pipeline`] runs a conservative WGSL pre-filter and invokes Naga only when the active
//! pipeline could sample one texture with multiple samplers or exceed GL's 16-slot texture table.
//! Extra `(texture, sampler)` pairs are lowered in place on `naga::Module` IR onto synthetic alias
//! texture bindings in the same bind group; [`crate::gpu::Session`] binds the same `TextureView`
//! to each alias.

use std::collections::{BTreeMap, BTreeSet};

use wgpu::naga::{
    self, AddressSpace, Block, Expression, Function, GlobalVariable, Handle, ImageClass,
    MemoryDecorations, Module, ResourceBinding, ShaderStage, Span, Statement, TypeInner,
    valid::{Capabilities, FunctionInfo, ModuleInfo, ValidationFlags, Validator},
};

use crate::ShaderError;

/// `wgpu-hal::gles::MAX_TEXTURE_SLOTS`: pipeline-wide GL texture-unit table length.
pub(crate) const GL_MAX_TEXTURE_SLOTS: usize = 16;

/// A synthetic texture binding added on `wgpu::Backend::Gl` for an extra sampler.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TextureAlias {
    pub group: u32,
    pub original: u32,
    pub alias: u32,
}

/// Result of pipeline-aware GL multi-sampler lowering.
#[derive(Debug, Default)]
pub(crate) struct LoweredPipeline {
    /// Rewritten vertex module (also shared by fragment when `same_module` is true).
    pub vertex: Option<Module>,
    /// Rewritten fragment module when separate from vertex.
    pub fragment: Option<Module>,
    /// Alias texture bindings needed by the pipeline's bind groups.
    pub aliases: Vec<TextureAlias>,
}

/// Maps the adapter's downlevel flags to `naga::valid::Capabilities` for a default (`Features::empty()`)
/// device, matching `wgpu_naga_bridge::features_to_naga_capabilities`.
pub(crate) fn naga_capabilities(downlevel: wgpu::DownlevelFlags) -> Capabilities {
    let mut caps = Capabilities::empty();
    caps.set(
        Capabilities::SHADER_FLOAT16_IN_FLOAT32,
        downlevel.contains(wgpu::DownlevelFlags::SHADER_F16_IN_F32),
    );
    caps.set(
        Capabilities::MULTISAMPLED_SHADING,
        downlevel.contains(wgpu::DownlevelFlags::MULTISAMPLED_SHADING),
    );
    caps.set(
        Capabilities::CUBE_ARRAY_TEXTURES,
        downlevel.contains(wgpu::DownlevelFlags::CUBE_ARRAY_TEXTURES),
    );
    caps
}

/// Conservative O(n) pre-filter: returns `false` only when valid WGSL cannot sample a texture
/// with two distinct samplers or exceed [`GL_MAX_TEXTURE_SLOTS`] across active stages.
///
/// Every WGSL texture/sampler global must carry `@binding` and end with `;`. Unusual syntax
/// (whitespace/comments between `@` and an attribute name, or comments inside `@binding ... ;`)
/// returns `true` so exact Naga analysis decides.
pub(crate) fn may_need_lowering(vs_code: &str, fs_code: Option<&str>, has_fragment: bool) -> bool {
    let Some((vs_samplers, vs_textures)) = scan_module_bindings(vs_code) else {
        return true;
    };
    let (fs_samplers, fs_textures) = match fs_code {
        Some(code) => match scan_module_bindings(code) {
            Some(counts) => counts,
            None => return true,
        },
        None => (0, 0),
    };
    let samplers = vs_samplers + fs_samplers;
    let textures = vs_textures + fs_textures;
    textures >= 1 && (samplers >= 2 || (has_fragment && textures > GL_MAX_TEXTURE_SLOTS))
}

fn scan_module_bindings(code: &str) -> Option<(usize, usize)> {
    let bytes = code.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'@' && !bytes.get(i + 1).is_some_and(u8::is_ascii_alphabetic) {
            return None;
        }
    }
    let has_alias = contains_ident(code, "alias");
    let mut samplers = 0usize;
    let mut textures = 0usize;
    let mut rest = code;
    while let Some(pos) = rest.find("@binding") {
        let after = &rest[pos + "@binding".len()..];
        let semi = after.find(';')?;
        let decl = &after[..semi];
        if decl.contains("//") || decl.contains("/*") || decl.contains("*/") {
            return None;
        }
        if has_alias {
            samplers += 1;
            textures += 1;
        } else {
            if decl.contains("sampler") {
                samplers += 1;
            }
            if decl.contains("texture_") {
                textures += 1;
            }
        }
        rest = &after[semi + 1..];
    }
    Some((samplers, textures))
}

fn contains_ident(text: &str, ident: &str) -> bool {
    let bytes = text.as_bytes();
    let is_ident = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    let mut offset = 0;
    while let Some(pos) = text[offset..].find(ident) {
        let start = offset + pos;
        let end = start + ident.len();
        let before_ok = start == 0 || !is_ident(bytes[start - 1]);
        let after_ok = end == bytes.len() || !is_ident(bytes[end]);
        if before_ok && after_ok {
            return true;
        }
        offset = end;
    }
    false
}

/// Lowers multi-sampler texture uses for a paired render pipeline on `wgpu::Backend::Gl`.
///
/// Ordinary pipelines bypass Naga completely via [`may_need_lowering`]. Invalid WGSL or missing
/// entry points return an empty [`LoweredPipeline`] so `wgpu`'s stock modules report diagnostics.
pub(crate) fn lower_pipeline(
    vs_code: &str,
    vertex_entry: Option<&str>,
    fragment: Option<(&str, Option<&str>)>,
    same_module: bool,
    limits: &wgpu::Limits,
    caps: Capabilities,
) -> Result<LoweredPipeline, ShaderError> {
    let distinct_fs_code = if same_module {
        None
    } else {
        fragment.map(|(code, _)| code)
    };
    if !may_need_lowering(vs_code, distinct_fs_code, fragment.is_some()) {
        return Ok(LoweredPipeline::default());
    }

    let Some((vs_mod, vs_info)) = parse_and_validate(vs_code, caps, limits.max_bind_groups) else {
        return Ok(LoweredPipeline::default());
    };
    let fs_parsed = match distinct_fs_code {
        Some(code) => {
            let Some(parsed) = parse_and_validate(code, caps, limits.max_bind_groups) else {
                return Ok(LoweredPipeline::default());
            };
            Some(parsed)
        }
        None => None,
    };

    let Some(vs_idx) = resolve_entry(&vs_mod, ShaderStage::Vertex, vertex_entry) else {
        return Ok(LoweredPipeline::default());
    };
    let fs_idx = match fragment {
        Some((_, frag_entry)) => {
            let fs_mod = fs_parsed.as_ref().map_or(&vs_mod, |(m, _)| m);
            let Some(idx) = resolve_entry(fs_mod, ShaderStage::Fragment, frag_entry) else {
                return Ok(LoweredPipeline::default());
            };
            Some(idx)
        }
        None => None,
    };

    let mut samplers_by_tex: BTreeMap<ResourceBinding, BTreeSet<ResourceBinding>> = BTreeMap::new();
    collect_stage_samplings(
        &vs_mod,
        vs_info.get_entry_point(vs_idx),
        &mut samplers_by_tex,
    );
    if let Some(idx) = fs_idx {
        let (fs_mod, fs_info) = fs_parsed
            .as_ref()
            .map_or((&vs_mod, &vs_info), |(m, i)| (m, i));
        collect_stage_samplings(fs_mod, fs_info.get_entry_point(idx), &mut samplers_by_tex);
    }

    let needs_split = samplers_by_tex.values().any(|s| s.len() >= 2);
    if !needs_split {
        let vs_tex = sampled_texture_bindings(&vs_mod, vs_info.get_entry_point(vs_idx));
        let fs_tex = fs_idx.map(|idx| {
            let (fs_mod, fs_info) = fs_parsed
                .as_ref()
                .map_or((&vs_mod, &vs_info), |(m, i)| (m, i));
            sampled_texture_bindings(fs_mod, fs_info.get_entry_point(idx))
        });
        check_texture_limits(&vs_tex, fs_tex.as_ref(), limits)?;
        return Ok(LoweredPipeline::default());
    }

    let mut occupied: BTreeMap<u32, BTreeSet<u32>> = BTreeMap::new();
    collect_all_bindings(&vs_mod, &mut occupied);
    if let Some((fs_mod, _)) = &fs_parsed {
        collect_all_bindings(fs_mod, &mut occupied);
    }

    let mut pair_alias: BTreeMap<(ResourceBinding, ResourceBinding), ResourceBinding> =
        BTreeMap::new();
    let mut allocated: Vec<(ResourceBinding, ResourceBinding)> = Vec::new();
    for (tex_rb, samplers) in &samplers_by_tex {
        if samplers.len() < 2 {
            continue;
        }
        // Keep the first sampler on the original texture binding so non-sampling uses
        // (`textureDimensions`, `textureLoad`) and that sampler stay untouched.
        for &smp_rb in samplers.iter().skip(1) {
            let alias_binding = allocate_binding(
                occupied.entry(tex_rb.group).or_default(),
                limits.max_bindings_per_bind_group,
                tex_rb.group,
            )?;
            let alias_rb = ResourceBinding {
                group: tex_rb.group,
                binding: alias_binding,
            };
            pair_alias.insert((*tex_rb, smp_rb), alias_rb);
            allocated.push((*tex_rb, alias_rb));
        }
    }

    let mut lowered_vs: Option<(Module, ModuleInfo)> = None;
    let mut lowered_fs: Option<(Module, ModuleInfo)> = None;

    if same_module {
        let mut active_eps = vec![vs_idx];
        if let Some(idx) = fs_idx
            && idx != vs_idx
        {
            active_eps.push(idx);
        }
        let rewritten = rewrite_module(vs_mod.clone(), &vs_info, &active_eps, &pair_alias)?;
        let info = validate(&rewritten, caps).map_err(|e| {
            ShaderError::gpu(format!(
                "GL multi-sampler lowering produced invalid Naga IR: {e}"
            ))
        })?;
        lowered_vs = Some((rewritten, info));
    } else {
        if stage_uses_any_alias(&vs_mod, vs_info.get_entry_point(vs_idx), &pair_alias) {
            let rewritten = rewrite_module(vs_mod.clone(), &vs_info, &[vs_idx], &pair_alias)?;
            let info = validate(&rewritten, caps).map_err(|e| {
                ShaderError::gpu(format!(
                    "GL multi-sampler lowering (vertex) produced invalid Naga IR: {e}"
                ))
            })?;
            lowered_vs = Some((rewritten, info));
        }
        if let (Some(idx), Some((fs_mod, fs_info))) = (fs_idx, &fs_parsed)
            && stage_uses_any_alias(fs_mod, fs_info.get_entry_point(idx), &pair_alias)
        {
            let rewritten = rewrite_module(fs_mod.clone(), fs_info, &[idx], &pair_alias)?;
            let info = validate(&rewritten, caps).map_err(|e| {
                ShaderError::gpu(format!(
                    "GL multi-sampler lowering (fragment) produced invalid Naga IR: {e}"
                ))
            })?;
            lowered_fs = Some((rewritten, info));
        }
    }

    let (eff_vs_mod, eff_vs_info) = lowered_vs
        .as_ref()
        .map_or((&vs_mod, &vs_info), |(m, i)| (m, i));
    let eff_vs_ep = eff_vs_info.get_entry_point(vs_idx);
    let mut post_samplers: BTreeMap<ResourceBinding, ResourceBinding> = BTreeMap::new();
    check_stage_samplings(eff_vs_mod, eff_vs_ep, &mut post_samplers)?;
    let vs_used = used_bindings(eff_vs_mod, eff_vs_ep);
    let vs_textures = sampled_texture_bindings(eff_vs_mod, eff_vs_ep);

    let (fs_used, fs_textures) = match fs_idx {
        Some(idx) => {
            let (eff_fs_mod, eff_fs_info) = if same_module {
                (eff_vs_mod, eff_vs_info)
            } else if let Some((m, i)) = &lowered_fs {
                (m, i)
            } else if let Some((m, i)) = &fs_parsed {
                (m, i)
            } else {
                (eff_vs_mod, eff_vs_info)
            };
            let eff_fs_ep = eff_fs_info.get_entry_point(idx);
            check_stage_samplings(eff_fs_mod, eff_fs_ep, &mut post_samplers)?;
            (
                used_bindings(eff_fs_mod, eff_fs_ep),
                Some(sampled_texture_bindings(eff_fs_mod, eff_fs_ep)),
            )
        }
        None => (BTreeSet::new(), None),
    };

    check_texture_limits(&vs_textures, fs_textures.as_ref(), limits)?;

    let mut aliases = Vec::new();
    for (orig_rb, alias_rb) in allocated {
        if vs_used.contains(&alias_rb) || fs_used.contains(&alias_rb) {
            aliases.push(TextureAlias {
                group: orig_rb.group,
                original: orig_rb.binding,
                alias: alias_rb.binding,
            });
        }
    }

    Ok(LoweredPipeline {
        vertex: lowered_vs.map(|(m, _)| m),
        fragment: lowered_fs.map(|(m, _)| m),
        aliases,
    })
}

fn parse_and_validate(
    code: &str,
    caps: Capabilities,
    max_bind_groups: u32,
) -> Option<(Module, ModuleInfo)> {
    let mut options = naga::front::wgsl::Options::new();
    options.capabilities = caps;
    let module = naga::front::wgsl::Frontend::new_with_options(options)
        .parse(code)
        .ok()?;
    let groups_ok = module
        .global_variables
        .iter()
        .filter_map(|(_, var)| var.binding)
        .all(|rb| rb.group < max_bind_groups);
    if !groups_ok {
        return None;
    }
    let info = validate(&module, caps).ok()?;
    Some((module, info))
}

fn validate(
    module: &Module,
    caps: Capabilities,
) -> Result<ModuleInfo, Box<naga::WithSpan<naga::valid::ValidationError>>> {
    Validator::new(ValidationFlags::all(), caps)
        .validate(module)
        .map_err(Box::new)
}

fn resolve_entry(module: &Module, stage: ShaderStage, name: Option<&str>) -> Option<usize> {
    match name {
        Some(wanted) => module
            .entry_points
            .iter()
            .position(|ep| ep.stage == stage && ep.name == wanted),
        None => {
            let mut matches = module
                .entry_points
                .iter()
                .enumerate()
                .filter(|(_, ep)| ep.stage == stage)
                .map(|(idx, _)| idx);
            match (matches.next(), matches.next()) {
                (Some(idx), None) => Some(idx),
                _ => None,
            }
        }
    }
}

fn is_sampled_or_depth_image(ty: &TypeInner) -> bool {
    matches!(
        *ty,
        TypeInner::Image {
            class: ImageClass::Sampled { .. } | ImageClass::Depth { .. },
            ..
        }
    )
}

fn is_handle_resource(ty: &TypeInner) -> bool {
    matches!(*ty, TypeInner::Image { .. } | TypeInner::Sampler { .. })
}

fn collect_all_bindings(module: &Module, out: &mut BTreeMap<u32, BTreeSet<u32>>) {
    for (_, var) in module.global_variables.iter() {
        if let Some(rb) = var.binding {
            out.entry(rb.group).or_default().insert(rb.binding);
        }
    }
}

fn used_bindings(module: &Module, ep_info: &FunctionInfo) -> BTreeSet<ResourceBinding> {
    module
        .global_variables
        .iter()
        .filter(|&(gv, _)| !ep_info[gv].is_empty())
        .filter_map(|(_, var)| var.binding)
        .collect()
}

fn sampled_texture_bindings(module: &Module, ep_info: &FunctionInfo) -> BTreeSet<ResourceBinding> {
    module
        .global_variables
        .iter()
        .filter(|&(gv, var)| {
            !ep_info[gv].is_empty() && is_sampled_or_depth_image(&module.types[var.ty].inner)
        })
        .filter_map(|(_, var)| var.binding)
        .collect()
}

fn collect_stage_samplings(
    module: &Module,
    ep_info: &FunctionInfo,
    out: &mut BTreeMap<ResourceBinding, BTreeSet<ResourceBinding>>,
) {
    for key in &ep_info.sampling_set {
        let Some(tex_rb) = module.global_variables[key.image].binding else {
            continue;
        };
        let Some(smp_rb) = module.global_variables[key.sampler].binding else {
            continue;
        };
        out.entry(tex_rb).or_default().insert(smp_rb);
    }
}

fn stage_uses_any_alias(
    module: &Module,
    ep_info: &FunctionInfo,
    pair_alias: &BTreeMap<(ResourceBinding, ResourceBinding), ResourceBinding>,
) -> bool {
    ep_info.sampling_set.iter().any(|key| {
        let Some(tex_rb) = module.global_variables[key.image].binding else {
            return false;
        };
        let Some(smp_rb) = module.global_variables[key.sampler].binding else {
            return false;
        };
        pair_alias.contains_key(&(tex_rb, smp_rb))
    })
}

fn check_stage_samplings(
    module: &Module,
    ep_info: &FunctionInfo,
    seen: &mut BTreeMap<ResourceBinding, ResourceBinding>,
) -> Result<(), ShaderError> {
    for key in &ep_info.sampling_set {
        let Some(tex_rb) = module.global_variables[key.image].binding else {
            continue;
        };
        let Some(smp_rb) = module.global_variables[key.sampler].binding else {
            continue;
        };
        if let Some(&prev) = seen.get(&tex_rb)
            && prev != smp_rb
        {
            return Err(ShaderError::gpu(format!(
                "OpenGL multi-sampler lowering could not separate texture binding \
                 ({}, {}) across samplers ({}, {}) and ({}, {})",
                tex_rb.group,
                tex_rb.binding,
                prev.group,
                prev.binding,
                smp_rb.group,
                smp_rb.binding
            )));
        }
        seen.insert(tex_rb, smp_rb);
    }
    Ok(())
}

fn check_texture_limits(
    vs_textures: &BTreeSet<ResourceBinding>,
    fs_textures: Option<&BTreeSet<ResourceBinding>>,
    limits: &wgpu::Limits,
) -> Result<(), ShaderError> {
    let per_stage = limits.max_sampled_textures_per_shader_stage as usize;
    let mut all_textures: BTreeSet<ResourceBinding> = BTreeSet::new();
    for (stage_name, textures) in [("vertex", Some(vs_textures)), ("fragment", fs_textures)] {
        let Some(textures) = textures else {
            continue;
        };
        if textures.len() > per_stage {
            return Err(ShaderError::gpu(format!(
                "{stage_name} stage uses {} sampled textures after GL multi-sampler lowering, \
                 exceeding device limit {per_stage}",
                textures.len()
            )));
        }
        all_textures.extend(textures.iter().copied());
    }
    if all_textures.len() > GL_MAX_TEXTURE_SLOTS {
        return Err(ShaderError::gpu(format!(
            "pipeline uses {} sampled texture slots on OpenGL, exceeding GL limit \
             {GL_MAX_TEXTURE_SLOTS}",
            all_textures.len()
        )));
    }
    Ok(())
}

fn allocate_binding(
    occupied: &mut BTreeSet<u32>,
    limit: u32,
    group: u32,
) -> Result<u32, ShaderError> {
    let preferred = occupied
        .iter()
        .next_back()
        .and_then(|&max| max.checked_add(1))
        .filter(|&cand| cand < limit);
    let chosen = preferred.or_else(|| (0..limit).find(|b| !occupied.contains(b)));
    let binding = chosen.ok_or_else(|| {
        ShaderError::gpu(format!(
            "bind group {group} has no free binding below device limit {limit} \
             for OpenGL multi-sampler alias"
        ))
    })?;
    occupied.insert(binding);
    Ok(binding)
}

/// Adds synthetic alias `GlobalVariable`s for pairs referenced in `active_eps` and rewrites those
/// entry points in place (specializing helper functions as needed).
fn rewrite_module(
    mut module: Module,
    info: &ModuleInfo,
    active_eps: &[usize],
    pair_alias: &BTreeMap<(ResourceBinding, ResourceBinding), ResourceBinding>,
) -> Result<Module, ShaderError> {
    // Resolve global handles directly from each active entry point's `sampling_set` so other
    // entry points in a multi-pass module that reuse `(group, binding)` never collide.
    let mut pair_alias_gv: BTreeMap<
        (Handle<GlobalVariable>, Handle<GlobalVariable>),
        Handle<GlobalVariable>,
    > = BTreeMap::new();
    let mut alias_gv_by_source: BTreeMap<
        (Handle<GlobalVariable>, ResourceBinding),
        Handle<GlobalVariable>,
    > = BTreeMap::new();

    for &ep_idx in active_eps {
        for key in &info.get_entry_point(ep_idx).sampling_set {
            let Some(tex_rb) = module.global_variables[key.image].binding else {
                continue;
            };
            let Some(smp_rb) = module.global_variables[key.sampler].binding else {
                continue;
            };
            let Some(&alias_rb) = pair_alias.get(&(tex_rb, smp_rb)) else {
                continue;
            };
            let alias_gv = *alias_gv_by_source
                .entry((key.image, alias_rb))
                .or_insert_with(|| {
                    let orig_ty = module.global_variables[key.image].ty;
                    module.global_variables.append(
                        GlobalVariable {
                            name: Some(format!(
                                "figstudio_gl_tex_{}_{}",
                                alias_rb.group, alias_rb.binding
                            )),
                            space: AddressSpace::Handle,
                            binding: Some(alias_rb),
                            ty: orig_ty,
                            init: None,
                            memory_decorations: MemoryDecorations::empty(),
                        },
                        Span::UNDEFINED,
                    )
                });
            pair_alias_gv.insert((key.image, key.sampler), alias_gv);
        }
    }

    let mut rewriter = ModuleRewriter {
        module: &mut module,
        pair_alias_gv: &pair_alias_gv,
        spec_cache: BTreeMap::new(),
        next_spec_id: 0,
    };

    for &ep_idx in active_eps {
        let mut ep_fn = std::mem::take(&mut rewriter.module.entry_points[ep_idx].function);
        rewriter.rewrite_function(&mut ep_fn, &[])?;
        rewriter.module.entry_points[ep_idx].function = ep_fn;
    }

    Ok(module)
}

type HandleEnv = Vec<(u32, Handle<GlobalVariable>)>;

struct ModuleRewriter<'a> {
    module: &'a mut Module,
    pair_alias_gv:
        &'a BTreeMap<(Handle<GlobalVariable>, Handle<GlobalVariable>), Handle<GlobalVariable>>,
    spec_cache: BTreeMap<(Handle<Function>, HandleEnv), Option<Handle<Function>>>,
    next_spec_id: u32,
}

impl ModuleRewriter<'_> {
    fn specialize(
        &mut self,
        orig_callee: Handle<Function>,
        env: HandleEnv,
    ) -> Result<Option<Handle<Function>>, ShaderError> {
        let key = (orig_callee, env.clone());
        if let Some(&cached) = self.spec_cache.get(&key) {
            return Ok(cached);
        }
        let mut spec = self.module.functions[orig_callee].clone();
        let changed = self.rewrite_function(&mut spec, &env)?;
        if !changed {
            self.spec_cache.insert(key, None);
            return Ok(None);
        }
        let base_name = spec.name.as_deref().unwrap_or("helper");
        let id = self.next_spec_id;
        self.next_spec_id += 1;
        spec.name = Some(format!("{base_name}_gl_alias_{id}"));
        let span = self.module.functions.get_span(orig_callee);
        let spec_handle = self.module.functions.append(spec, span);
        self.spec_cache.insert(key, Some(spec_handle));
        Ok(Some(spec_handle))
    }

    /// Rewrites `func` in place under handle-argument bindings `env`.
    ///
    /// Mutates `Expression::GlobalVariable` handles in place so no expression handles shift.
    /// Fails explicitly if one expression handle is shared across `ImageSample` uses that require
    /// different texture globals (for example a helper taking one texture parameter and sampling it
    /// with two different global samplers inside the same function body).
    fn rewrite_function(
        &mut self,
        func: &mut Function,
        env: &[(u32, Handle<GlobalVariable>)],
    ) -> Result<bool, ShaderError> {
        let mut changed = false;

        if !env.is_empty() {
            let env_map: BTreeMap<u32, Handle<GlobalVariable>> = env.iter().copied().collect();
            let mut new_args = Vec::with_capacity(func.arguments.len());
            let mut arg_remap: Vec<Option<u32>> = Vec::with_capacity(func.arguments.len());
            for (i, arg) in func.arguments.drain(..).enumerate() {
                if env_map.contains_key(&(i as u32)) {
                    arg_remap.push(None);
                } else {
                    arg_remap.push(Some(new_args.len() as u32));
                    new_args.push(arg);
                }
            }
            func.arguments = new_args;
            for (_, expr) in func.expressions.iter_mut() {
                if let Expression::FunctionArgument(idx) = *expr {
                    if let Some(&gv) = env_map.get(&idx) {
                        *expr = Expression::GlobalVariable(gv);
                    } else if let Some(Some(new_idx)) = arg_remap.get(idx as usize) {
                        *expr = Expression::FunctionArgument(*new_idx);
                    }
                }
            }
        }

        let mut body = std::mem::take(&mut func.body);
        if self.rewrite_block_calls(&mut body, &mut func.expressions)? {
            changed = true;
        }
        func.body = body;

        let mut desired: BTreeMap<Handle<Expression>, Handle<GlobalVariable>> = BTreeMap::new();
        for (_, expr) in func.expressions.iter() {
            if let Expression::ImageSample { image, sampler, .. } = *expr
                && let (Expression::GlobalVariable(img_gv), Expression::GlobalVariable(smp_gv)) =
                    (&func.expressions[image], &func.expressions[sampler])
            {
                let target_gv = self
                    .pair_alias_gv
                    .get(&(*img_gv, *smp_gv))
                    .copied()
                    .unwrap_or(*img_gv);
                if let Some(&prev_gv) = desired.get(&image)
                    && prev_gv != target_gv
                {
                    return Err(ShaderError::gpu(
                        "OpenGL multi-sampler lowering cannot split a texture handle expression \
                         shared across multiple samplers in one function",
                    ));
                }
                desired.insert(image, target_gv);
            }
        }

        for (expr_h, target_gv) in desired {
            if let Expression::GlobalVariable(ref mut current_gv) = func.expressions[expr_h]
                && *current_gv != target_gv
            {
                *current_gv = target_gv;
                changed = true;
            }
        }

        Ok(changed)
    }

    fn rewrite_block_calls(
        &mut self,
        block: &mut Block,
        expressions: &mut naga::Arena<Expression>,
    ) -> Result<bool, ShaderError> {
        let mut changed = false;
        for stmt in block.iter_mut() {
            match stmt {
                Statement::Call {
                    function,
                    arguments,
                    result,
                } => {
                    let mut call_env: HandleEnv = Vec::new();
                    for (arg_idx, &arg_expr) in arguments.iter().enumerate() {
                        if let Expression::GlobalVariable(gv) = expressions[arg_expr]
                            && is_handle_resource(
                                &self.module.types[self.module.global_variables[gv].ty].inner,
                            )
                        {
                            call_env.push((arg_idx as u32, gv));
                        }
                    }
                    if let Some(spec_fn) = self.specialize(*function, call_env.clone())? {
                        *function = spec_fn;
                        if !call_env.is_empty() {
                            let stripped: BTreeSet<usize> =
                                call_env.iter().map(|&(i, _)| i as usize).collect();
                            let mut idx = 0usize;
                            arguments.retain(|_| {
                                let keep = !stripped.contains(&idx);
                                idx += 1;
                                keep
                            });
                        }
                        if let Some(res_h) = *result
                            && let Expression::CallResult(ref mut callee_ref) = expressions[res_h]
                        {
                            *callee_ref = spec_fn;
                        }
                        changed = true;
                    }
                }
                Statement::Block(inner) => {
                    changed |= self.rewrite_block_calls(inner, expressions)?;
                }
                Statement::If { accept, reject, .. } => {
                    changed |= self.rewrite_block_calls(accept, expressions)?;
                    changed |= self.rewrite_block_calls(reject, expressions)?;
                }
                Statement::Switch { cases, .. } => {
                    for case in cases {
                        changed |= self.rewrite_block_calls(&mut case.body, expressions)?;
                    }
                }
                Statement::Loop {
                    body, continuing, ..
                } => {
                    changed |= self.rewrite_block_calls(body, expressions)?;
                    changed |= self.rewrite_block_calls(continuing, expressions)?;
                }
                _ => {}
            }
        }
        Ok(changed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_limits() -> wgpu::Limits {
        wgpu::Limits::default()
    }

    fn default_caps() -> Capabilities {
        Capabilities::empty()
    }

    fn assert_single_sampler_per_texture(module: &Module, stage: ShaderStage, name: &str) {
        let info = validate(module, default_caps()).expect("lowered Naga module validates");
        let ep_idx = module
            .entry_points
            .iter()
            .position(|ep| ep.stage == stage && ep.name == name)
            .expect("entry point exists");
        let mut seen: BTreeMap<ResourceBinding, ResourceBinding> = BTreeMap::new();
        for key in &info.get_entry_point(ep_idx).sampling_set {
            let tex_rb = module.global_variables[key.image].binding.unwrap();
            let smp_rb = module.global_variables[key.sampler].binding.unwrap();
            if let Some(&prev) = seen.get(&tex_rb) {
                assert_eq!(
                    prev, smp_rb,
                    "entry point {name} still samples {tex_rb:?} with both {prev:?} and {smp_rb:?}"
                );
            }
            seen.insert(tex_rb, smp_rb);
        }
    }

    #[test]
    fn prefilter_bypasses_ordinary_single_sampler_and_helper_param_shaders() {
        let single_with_helper_params = r"
// anti-aliasing comment must not trigger the type-renaming fallback
@group(0) @binding(0) var<uniform> u: vec4f;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var tex0: texture_2d<f32>;
@group(0) @binding(3) var tex1: texture_2d<f32>;
fn helper(t: texture_2d<f32>, s: sampler, uv: vec2f) -> vec4f {
  return textureSampleLevel(t, s, uv, 0.0);
}
@vertex fn vs() -> @builtin(position) vec4f { return vec4f(0.0); }
@fragment fn fs() -> @location(0) vec4f { return helper(tex0, samp, vec2f(0.5)) + u; }
";
        assert!(!may_need_lowering(single_with_helper_params, None, true));

        // Type-aliased samplers must NOT be missed by the pre-filter.
        let aliased_samplers = r"
alias S = sampler;
@group(0) @binding(0) var tex: texture_2d<f32>;
@group(0) @binding(1) var s0: S;
@group(0) @binding(2) var s1: S;
@vertex fn vs() -> @builtin(position) vec4f { return vec4f(0.0); }
@fragment fn fs() -> @location(0) vec4f {
  return textureSampleLevel(tex, s0, vec2f(0.5), 0.0)
       + textureSampleLevel(tex, s1, vec2f(0.5), 0.0);
}
";
        assert!(may_need_lowering(aliased_samplers, None, true));
        let lowered = lower_pipeline(
            aliased_samplers,
            Some("vs"),
            Some((aliased_samplers, Some("fs"))),
            true,
            &default_limits(),
            default_caps(),
        )
        .expect("aliased dual samplers lower cleanly");
        assert_eq!(lowered.aliases.len(), 1);
        assert_single_sampler_per_texture(
            lowered.vertex.as_ref().expect("rewritten"),
            ShaderStage::Fragment,
            "fs",
        );
    }

    #[test]
    fn multipass_single_module_reusing_bindings_across_entry_points_lowers_cleanly() {
        let wgsl = r"
@group(0) @binding(0) var pass1_tex: texture_2d<f32>;
@group(0) @binding(1) var s0: sampler;
@group(0) @binding(2) var s1: sampler;
@group(0) @binding(0) var<uniform> pass2_u: vec4f;
@group(0) @binding(1) var pass2_tex: texture_2d<f32>;
@group(0) @binding(2) var pass2_s: sampler;

@vertex fn vs() -> @builtin(position) vec4f { return vec4f(0.0); }
@fragment fn fs_pass1() -> @location(0) vec4f {
  return textureSampleLevel(pass1_tex, s0, vec2f(0.25), 0.0)
       + textureSampleLevel(pass1_tex, s1, vec2f(0.75), 0.0);
}
@fragment fn fs_pass2() -> @location(0) vec4f {
  return textureSampleLevel(pass2_tex, pass2_s, vec2f(0.5), 0.0) * pass2_u;
}
";
        let pass1 = lower_pipeline(
            wgsl,
            Some("vs"),
            Some((wgsl, Some("fs_pass1"))),
            true,
            &default_limits(),
            default_caps(),
        )
        .expect("pass1 lowers despite pass2 reusing @binding(0) as uniform");
        let module = pass1.vertex.expect("pass1 rewritten");
        assert_single_sampler_per_texture(&module, ShaderStage::Fragment, "fs_pass1");
        assert_eq!(
            pass1.aliases,
            vec![TextureAlias {
                group: 0,
                original: 0,
                alias: 3,
            }]
        );

        let pass2 = lower_pipeline(
            wgsl,
            Some("vs"),
            Some((wgsl, Some("fs_pass2"))),
            true,
            &default_limits(),
            default_caps(),
        )
        .expect("pass2 stays unadapted");
        assert!(pass2.vertex.is_none());
        assert!(pass2.aliases.is_empty());
    }

    #[test]
    fn shared_handle_parameter_with_two_global_samplers_errors_explicitly() {
        let wgsl = r"
@group(0) @binding(0) var tex: texture_2d<f32>;
@group(0) @binding(1) var s0: sampler;
@group(0) @binding(2) var s1: sampler;
fn sample_both(t: texture_2d<f32>, uv: vec2f) -> vec4f {
  return textureSampleLevel(t, s0, uv, 0.0) + textureSampleLevel(t, s1, uv, 0.0);
}
@vertex fn vs() -> @builtin(position) vec4f { return vec4f(0.0); }
@fragment fn fs() -> @location(0) vec4f { return sample_both(tex, vec2f(0.5)); }
";
        let err = lower_pipeline(
            wgsl,
            Some("vs"),
            Some((wgsl, Some("fs"))),
            true,
            &default_limits(),
            default_caps(),
        )
        .expect_err("shared handle expression must error instead of silently corrupting");
        assert!(!err.is_script(), "must be a GPU error: {err}");
    }

    #[test]
    fn reuses_binding_gap_when_max_binding_reaches_device_limit() {
        let wgsl = r"
@group(0) @binding(999) var tex: texture_2d<f32>;
@group(0) @binding(1) var s0: sampler;
@group(0) @binding(2) var s1: sampler;
@vertex fn vs() -> @builtin(position) vec4f { return vec4f(0.0); }
@fragment fn fs() -> @location(0) vec4f {
  return textureSampleLevel(tex, s0, vec2f(0.5), 0.0)
       + textureSampleLevel(tex, s1, vec2f(0.5), 0.0);
}
";
        let limits = default_limits();
        let lowered = lower_pipeline(
            wgsl,
            Some("vs"),
            Some((wgsl, Some("fs"))),
            true,
            &limits,
            default_caps(),
        )
        .expect("gap reuse succeeds");
        assert_eq!(lowered.aliases.len(), 1);
        assert_eq!(lowered.aliases[0].alias, 0);
    }

    #[test]
    fn two_module_pipeline_avoids_other_module_bindings() {
        let vs_wgsl = r"
@group(0) @binding(0) var tex: texture_2d<f32>;
@group(0) @binding(1) var s0: sampler;
@vertex fn vs() -> @builtin(position) vec4f {
  return textureSampleLevel(tex, s0, vec2f(0.5), 0.0);
}
";
        let fs_wgsl = r"
@group(0) @binding(0) var tex: texture_2d<f32>;
@group(0) @binding(2) var s1: sampler;
@group(0) @binding(3) var<uniform> tint: vec4f;
@fragment fn fs() -> @location(0) vec4f {
  return textureSampleLevel(tex, s1, vec2f(0.5), 0.0) * tint;
}
";
        let lowered = lower_pipeline(
            vs_wgsl,
            Some("vs"),
            Some((fs_wgsl, Some("fs"))),
            false,
            &default_limits(),
            default_caps(),
        )
        .expect("cross-stage split succeeds");
        assert!(lowered.vertex.is_none());
        assert_single_sampler_per_texture(
            lowered.fragment.as_ref().expect("fragment rewritten"),
            ShaderStage::Fragment,
            "fs",
        );
        assert_eq!(
            lowered.aliases,
            vec![TextureAlias {
                group: 0,
                original: 0,
                alias: 4,
            }]
        );
    }

    #[test]
    fn rejects_pipeline_exceeding_gl_16_texture_slots() {
        let mut decls = String::new();
        let mut exprs = String::new();
        for i in 0..16 {
            decls.push_str(&format!(
                "@group(0) @binding({i}) var t{i}: texture_2d<f32>;\n"
            ));
            if i > 0 {
                exprs.push_str(" + ");
            }
            exprs.push_str(&format!("textureSampleLevel(t{i}, s0, vec2f(0.5), 0.0)"));
        }
        let wgsl = format!(
            "{decls}
@group(0) @binding(16) var s0: sampler;
@group(0) @binding(17) var s1: sampler;
@vertex fn vs() -> @builtin(position) vec4f {{ return vec4f(0.0); }}
@fragment fn fs() -> @location(0) vec4f {{
  return {exprs} + textureSampleLevel(t0, s1, vec2f(0.5), 0.0);
}}
"
        );
        let err = lower_pipeline(
            &wgsl,
            Some("vs"),
            Some((&wgsl, Some("fs"))),
            true,
            &default_limits(),
            default_caps(),
        )
        .expect_err("17 texture slots must be rejected on GL");
        assert!(!err.is_script(), "must be a GPU error: {err}");
        assert!(err.to_string().contains("16"), "{err}");
    }

    #[test]
    fn rejects_cross_stage_17_texture_slots_even_with_one_sampler() {
        let mut vs_decls = String::new();
        let mut vs_exprs = String::new();
        let mut fs_decls = String::new();
        let mut fs_exprs = String::new();
        for i in 0..9 {
            vs_decls.push_str(&format!(
                "@group(0) @binding({i}) var tv{i}: texture_2d<f32>;\n"
            ));
            if i > 0 {
                vs_exprs.push_str(" + ");
            }
            vs_exprs.push_str(&format!("textureSampleLevel(tv{i}, s0, vec2f(0.5), 0.0)"));
        }
        for i in 9..17 {
            fs_decls.push_str(&format!(
                "@group(0) @binding({i}) var tf{i}: texture_2d<f32>;\n"
            ));
            if i > 9 {
                fs_exprs.push_str(" + ");
            }
            fs_exprs.push_str(&format!("textureSampleLevel(tf{i}, s0, vec2f(0.5), 0.0)"));
        }
        let wgsl = format!(
            "{vs_decls}{fs_decls}
@group(0) @binding(20) var s0: sampler;
@vertex fn vs() -> @builtin(position) vec4f {{ return {vs_exprs}; }}
@fragment fn fs() -> @location(0) vec4f {{ return {fs_exprs}; }}
"
        );
        assert!(may_need_lowering(&wgsl, None, true));
        let err = lower_pipeline(
            &wgsl,
            Some("vs"),
            Some((&wgsl, Some("fs"))),
            true,
            &default_limits(),
            default_caps(),
        )
        .expect_err("17 cross-stage textures must be rejected on GL");
        assert!(!err.is_script(), "must be a GPU error: {err}");
        assert!(err.to_string().contains("16"), "{err}");
    }

    #[test]
    fn invalid_wgsl_or_disabled_capability_returns_default_without_panicking() {
        let bad = lower_pipeline(
            "fn nope(",
            Some("vs"),
            None,
            true,
            &default_limits(),
            default_caps(),
        )
        .expect("invalid WGSL defers to wgpu");
        assert!(bad.vertex.is_none());

        let f16 = r"
enable f16;
@group(0) @binding(0) var tex: texture_2d<f32>;
@group(0) @binding(1) var s0: sampler;
@group(0) @binding(2) var s1: sampler;
@vertex fn vs() -> @builtin(position) vec4f { return vec4f(0.0); }
@fragment fn fs() -> @location(0) vec4f {
  let x: f16 = 1.0h;
  return textureSampleLevel(tex, s0, vec2f(0.5), 0.0)
       + textureSampleLevel(tex, s1, vec2f(0.5), 0.0) * f32(x);
}
";
        let unenabled = lower_pipeline(
            f16,
            Some("vs"),
            Some((f16, Some("fs"))),
            true,
            &default_limits(),
            default_caps(),
        )
        .expect("disabled capability defers to wgpu");
        assert!(unenabled.vertex.is_none());
    }
}
