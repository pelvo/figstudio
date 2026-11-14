//! Wasm-safe MCP editing helpers (`get_editable`, `describe_edit_ops`, `copy_for_figma`).

use figstudio_fig::clipboard::encode_clipboard_html;
use figstudio_fig::edit::describe_edit_ops_markdown;
use figstudio_fig::{Document, Guid, Val};
use serde_json::{Value, json};

use crate::node_id::{NodeId, parse_node_id};
use crate::output::Output;

/// Returns structured documentation of all `EditOp` variants accepted by `apply_edits`.
pub fn describe_ops() -> Output {
    let md = describe_edit_ops_markdown();
    Output::structured(json!({
        "markdown": md,
        "operations": [
            "create_shape", "create_text", "create_page", "delete_node", "duplicate_node",
            "rename", "set_layer_props", "move", "resize", "rotate", "reparent",
            "set_corner_radii", "set_constraints", "set_auto_layout", "set_auto_layout_child",
            "set_solid_fill", "set_image_fill", "set_stroke", "set_effects",
            "set_text", "splice_text", "style_text_range", "set_typography",
            "set_instance_override", "reset_instance_override", "set_component_prop",
            "switch_variant", "swap_instance", "detach_instance", "create_component",
            "instantiate_component", "bind_style", "create_fill_style",
            "bind_paint_variable", "set_variable_mode", "set_variable_color",
            "set_vector_network", "boolean_combine", "patch_raw"
        ]
    }))
}

/// Inspects a single node (or resolved instance path `I1:2;3:4`) and returns all of its
/// editable layout, paint, typography, component, style, and variable fields as clean JSON.
pub fn editable(doc: &Document, target: &str) -> Output {
    let parsed = match parse_node_id(target) {
        Ok(id) => id,
        Err(e) => return Output::fault(e.to_string()),
    };

    let (guid, instance_outer, kind, name, parent, data) = match parsed {
        NodeId::Node(id) => {
            let Some(n) = doc.node(id) else {
                return Output::fault(format!("Node {id} does not exist"));
            };
            (
                id,
                None,
                n.kind.clone(),
                n.name.clone(),
                n.parent,
                n.data.clone(),
            )
        }
        NodeId::InstancePath(path) => {
            let outer = path.instance();
            match figstudio_fig::resolve::resolve_path(doc, &path) {
                Ok(resolved) => {
                    let base_kind = doc
                        .node(resolved.node)
                        .map_or_else(|| "UNKNOWN".to_string(), |n| n.kind.clone());
                    let nm = resolved.data.text("name").unwrap_or("").to_string();
                    (
                        resolved.node,
                        Some(outer),
                        base_kind,
                        nm,
                        None,
                        resolved.data,
                    )
                }
                Err(e) => return Output::fault(format!("{e:?}")),
            }
        }
    };

    let tf = figstudio_render::geom::affine(data.get("transform"));
    let coeffs = tf.as_coeffs();
    let rotation_deg = (-coeffs[1]).atan2(coeffs[0]).to_degrees();
    let (w, h) = figstudio_render::geom::size(&data);
    let page_bounds = figstudio_render::node_bounds(doc, instance_outer.unwrap_or(guid))
        .map(|r| json!({"x": r.x0, "y": r.y0, "width": r.width(), "height": r.height()}));

    let base_r = data.num("cornerRadius").unwrap_or(0.0);
    let radii = if data.flag("rectangleCornerRadiiIndependent") == Some(true) {
        [
            data.num("rectangleTopLeftCornerRadius").unwrap_or(base_r),
            data.num("rectangleTopRightCornerRadius").unwrap_or(base_r),
            data.num("rectangleBottomRightCornerRadius")
                .unwrap_or(base_r),
            data.num("rectangleBottomLeftCornerRadius")
                .unwrap_or(base_r),
        ]
    } else {
        [base_r; 4]
    };

    let auto_layout = data.text("stackMode").filter(|m| *m != "NONE").map(|mode| {
        json!({
            "mode": mode,
            "spacing": data.num("stackSpacing").unwrap_or(0.0),
            "counterSpacing": data.num("stackCounterSpacing"),
            "horizontalPadding": data.num("stackHorizontalPadding").or_else(|| data.num("stackPadding")).unwrap_or(0.0),
            "verticalPadding": data.num("stackVerticalPadding").or_else(|| data.num("stackPadding")).unwrap_or(0.0),
            "paddingRight": data.num("stackPaddingRight"),
            "paddingBottom": data.num("stackPaddingBottom"),
            "primarySizing": data.text("stackPrimarySizing"),
            "counterSizing": data.text("stackCounterSizing"),
            "primaryAlign": data.text("stackPrimaryAlignItems").or_else(|| data.text("stackJustify")),
            "counterAlign": data.text("stackCounterAlignItems").or_else(|| data.text("stackAlign")),
            "wrap": data.text("stackWrap") == Some("WRAP"),
        })
    });

    let text_info = (kind == "TEXT").then(|| {
        json!({
            "characters": data.path(&["textData", "characters"]).and_then(Val::as_str).unwrap_or(""),
            "fontFamily": data.path(&["fontName", "family"]).and_then(Val::as_str),
            "fontStyle": data.path(&["fontName", "style"]).and_then(Val::as_str),
            "fontSize": data.num("fontSize"),
            "alignHorizontal": data.text("textAlignHorizontal"),
            "alignVertical": data.text("textAlignVertical"),
            "autoResize": data.text("textAutoResize"),
            "styleRunsCount": data.path(&["textData", "styleOverrideTable"]).map_or(0, |t| t.items().len()),
        })
    });

    let instance_info = (kind == "INSTANCE").then(|| {
        let sym_id = figstudio_fig::resolve::symbol_id(&data);
        json!({
            "symbolId": sym_id.map(|g| g.to_string()),
            "symbolName": sym_id.and_then(|g| doc.node(g)).map(|n| n.name.as_str()),
            "overrideCount": data.path(&["symbolData", "symbolOverrides"]).map_or(0, |o| o.items().len()),
        })
    });

    Output::structured(json!({
        "nodeId": guid.to_string(),
        "enclosingInstance": instance_outer.map(|g| g.to_string()),
        "overrideKey": doc.node(guid).map(|n| n.override_key().to_string()),
        "name": name,
        "kind": kind,
        "parent": parent.map(|p| p.to_string()),
        "visible": data.flag("visible") != Some(false),
        "locked": data.flag("locked") == Some(true),
        "opacity": data.num("opacity").unwrap_or(1.0),
        "blendMode": data.text("blendMode").unwrap_or("PASS_THROUGH"),
        "clipContent": data.flag("frameMaskDisabled") == Some(false),
        "x": coeffs[4],
        "y": coeffs[5],
        "width": w,
        "height": h,
        "rotationDegrees": rotation_deg,
        "pageBounds": page_bounds,
        "cornerRadii": radii,
        "cornerSmoothing": data.num("cornerSmoothing").unwrap_or(0.0),
        "constraints": {
            "horizontal": data.text("horizontalConstraint").unwrap_or("MIN"),
            "vertical": data.text("verticalConstraint").unwrap_or("MIN"),
        },
        "autoLayout": auto_layout,
        "autoLayoutChild": {
            "primaryGrow": data.num("stackChildPrimaryGrow").unwrap_or(0.0),
            "alignSelf": data.text("stackChildAlignSelf").unwrap_or("AUTO"),
            "positioning": data.text("stackPositioning").unwrap_or("AUTO"),
        },
        "fills": summarize_paints(data.get("fillPaints")),
        "strokes": summarize_paints(data.get("strokePaints")),
        "strokeWeight": data.num("strokeWeight").unwrap_or(0.0),
        "strokeAlign": data.text("strokeAlign").unwrap_or("INSIDE"),
        "text": text_info,
        "instance": instance_info,
    }))
}

/// Encodes one or more node subtrees (including transitive local `SYMBOL` masters) as Figma
/// `text/html` clipboard markup ready for pasting into Figma.
pub fn copy_for_figma(doc: &Document, raw_ids: &[String], file_key: &str) -> Output {
    let mut roots: Vec<Guid> = Vec::with_capacity(raw_ids.len());
    for raw in raw_ids {
        match parse_node_id(raw) {
            Ok(NodeId::Node(id)) => roots.push(id),
            Ok(NodeId::InstancePath(p)) => roots.push(p.instance()),
            Err(e) => return Output::fault(e.to_string()),
        }
    }
    match encode_clipboard_html(doc, &roots, 1, Some(file_key)) {
        Ok(html) => Output::structured(json!({
            "fileKey": file_key,
            "rootIds": roots.iter().map(Guid::to_string).collect::<Vec<_>>(),
            "html": html,
            "bytes": html.len(),
        })),
        Err(e) => Output::fault(e.to_string()),
    }
}

fn summarize_paints(list: Option<&Val>) -> Vec<Value> {
    list.map(Val::items)
        .unwrap_or_default()
        .iter()
        .map(|p| {
            let kind = p.text("type").unwrap_or("UNKNOWN");
            let hex = p.get("color").and_then(|c| {
                let r = (c.num("r")? * 255.0).round() as u8;
                let g = (c.num("g")? * 255.0).round() as u8;
                let b = (c.num("b")? * 255.0).round() as u8;
                Some(format!("#{r:02X}{g:02X}{b:02X}"))
            });
            json!({
                "type": kind,
                "visible": p.flag("visible") != Some(false),
                "opacity": p.num("opacity").unwrap_or(1.0),
                "hex": hex,
                "hasVariable": p.get("colorVar").is_some(),
            })
        })
        .collect()
}
