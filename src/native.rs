//! Preparing scenes happens on the loading thread, before any GL work or desktop changes.

use std::path::Path;

use apolline_scene::{Prepared, Project};
use apolline_wall_protocol::steam;

pub fn load(
    folder: &Path,
    values: &apolline_scene::properties::Values,
) -> Result<Prepared, String> {
    let assets = steam::assets(folder);
    Project::open(folder, assets.as_deref())
        .and_then(|mut project| {
            project.set_properties(values)?;
            project.prepare()
        })
        .map_err(|err| format!("{}: {err}", folder.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn static_fragment_materials_reach_native_preparation_with_property_edits() {
        let folder =
            std::env::temp_dir().join(format!("wall-native-fragment-{}", std::process::id()));
        std::fs::create_dir_all(folder.join("shaders")).unwrap();
        std::fs::create_dir_all(folder.join("materials")).unwrap();
        std::fs::write(folder.join("project.json"), r#"{"general":{"properties":{"amount":{"type":"slider","value":1,"min":0,"max":1,"fraction":true}}}}"#).unwrap();
        std::fs::write(folder.join("scene.json"), r#"{"general":{"orthogonalprojection":{"width":10,"height":10}},"objects":[{"image":"model.json"}]}"#).unwrap();
        std::fs::write(folder.join("model.json"), r#"{"material":"material.json"}"#).unwrap();
        std::fs::write(folder.join("material.json"), r#"{"passes":[{"shader":"demo","textures":["same","same"],"constantshadervalues":{"amount":{"user":"amount","value":1}}}]}"#).unwrap();
        std::fs::write(folder.join("shaders/demo.frag"), "uniform float u_Amount; // {\"material\":\"amount\",\"default\":1}\nuniform sampler2D g_Texture0;\nuniform sampler2D g_Texture1;\nvarying vec2 v_TexCoord;\nvoid main() { gl_FragColor = mix(texSample2D(g_Texture0, v_TexCoord), texSample2D(g_Texture1, v_TexCoord), u_Amount); }").unwrap();
        std::fs::write(folder.join("materials/same.tex"), texture(0, &[255; 4])).unwrap();
        let plain = load(&folder, &Default::default()).unwrap();
        let edited = load(
            &folder,
            &apolline_scene::properties::Values::from([("amount".into(), serde_json::json!(0.25))]),
        )
        .unwrap();
        assert!(plain.layers[0].fragment.is_some());
        assert_eq!(plain.layers[0].secondary.len(), 1);
        assert_eq!(plain.layers[0].secondary[0].slot, 1);
        assert!(std::sync::Arc::ptr_eq(
            &plain.layers[0].texture,
            &plain.layers[0].secondary[0].texture
        ));
        assert_ne!(plain.layers[0].fragment, edited.layers[0].fragment);
        for (format, pixels) in [(9, vec![128]), (8, vec![128, 64])] {
            std::fs::write(folder.join("materials/same.tex"), texture(format, &pixels)).unwrap();
            let compact = load(&folder, &Default::default()).unwrap();
            assert_eq!(compact.layers[0].texture.images[0][0].pixels, pixels);
            assert!(std::sync::Arc::ptr_eq(
                &compact.layers[0].texture,
                &compact.layers[0].secondary[0].texture
            ));
        }
        std::fs::write(folder.join("shaders/demo.frag"), "uniform float g_Time;\nuniform float u_Amount; // {\"material\":\"amount\",\"default\":1}\nvoid main() { gl_FragColor = vec4(fract(g_Time) * u_Amount); }").unwrap();
        assert!(load(&folder, &Default::default()).is_ok());
        assert!(
            Project::open(&folder, None)
                .unwrap()
                .prepare_still()
                .is_err()
        );
        std::fs::write(folder.join("material.json"), r#"{"passes":[{"shader":"demo","textures":["same",null,null,null,null,null,null,"same"],"constantshadervalues":{"amount":{"user":"amount","value":1}}}]}"#).unwrap();
        std::fs::write(folder.join("shaders/demo.frag"), "uniform sampler2D g_Texture7; // {\"combo\":\"MASK\",\"mode\":\"opacitymask\"}\nuniform float u_Amount; // {\"material\":\"amount\",\"default\":1}\n#if MASK\nuniform float g_Time;\n#endif\nvoid main() { vec4 pixel = vec4(u_Amount);\n#if MASK\npixel *= texSample2D(g_Texture7, vec2(0.5)) * g_Time;\n#endif\ngl_FragColor = pixel; }").unwrap();
        let sparse = load(&folder, &Default::default()).unwrap();
        assert_eq!(sparse.layers[0].secondary[0].slot, 7);
        assert!(std::sync::Arc::ptr_eq(
            &sparse.layers[0].texture,
            &sparse.layers[0].secondary[0].texture
        ));
        std::fs::write(folder.join("material.json"), r#"{"passes":[{"shader":"demo","textures":["same"],"constantshadervalues":{"amount":{"user":"amount","value":1}}}]}"#).unwrap();
        let absent = load(&folder, &Default::default()).unwrap();
        assert!(absent.layers[0].secondary.is_empty());
        assert!(
            Project::open(&folder, None)
                .unwrap()
                .prepare_still()
                .is_ok()
        );
        std::fs::write(folder.join("material.json"), r#"{"passes":[{"shader":"demo","constantshadervalues":{"amount":{"user":"amount","value":1}}}]}"#).unwrap();
        std::fs::write(folder.join("shaders/demo.frag"), "uniform sampler2D g_Texture0; // {\"default\":\"same\"}\nuniform sampler2D g_Texture7; // {\"default\":\"same\",\"combo\":\"MASK\"}\nuniform float u_Amount; // {\"material\":\"amount\",\"default\":1}\nvoid main() { gl_FragColor = texSample2D(g_Texture0, vec2(0.5)) * texSample2D(g_Texture7, vec2(0.5)) * u_Amount; }").unwrap();
        let defaults = load(&folder, &Default::default()).unwrap();
        assert_eq!(defaults.layers[0].secondary[0].slot, 7);
        assert!(std::sync::Arc::ptr_eq(
            &defaults.layers[0].texture,
            &defaults.layers[0].secondary[0].texture
        ));
        let edited = load(
            &folder,
            &apolline_scene::properties::Values::from([("amount".into(), serde_json::json!(0.25))]),
        )
        .unwrap();
        assert_ne!(defaults.layers[0].fragment, edited.layers[0].fragment);
        std::fs::remove_dir_all(folder).unwrap();
    }

    fn texture(format: u32, pixels: &[u8]) -> Vec<u8> {
        let mut texture = b"TEXV0005\0TEXI0001\0".to_vec();
        for value in [format, 0, 1, 1, 1, 1, 0] {
            texture.extend(value.to_le_bytes());
        }
        texture.extend(b"TEXB0002\0");
        for value in [1u32, 1, 1, 1, 0, 0, pixels.len() as u32] {
            texture.extend(value.to_le_bytes());
        }
        texture.extend(pixels);
        texture
    }

    #[test]
    fn saved_property_values_reach_the_prepared_scene() {
        let folder = std::env::temp_dir().join(format!("wall-native-props-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("project.json"), r#"{"type":"scene","general":{"properties":{"color":{"type":"color","value":"1 0 0"}}}}"#).unwrap();
        std::fs::write(folder.join("scene.json"), r#"{"general":{"orthogonalprojection":{"width":100,"height":100},"clearcolor":{"user":"color","value":"0 0 0"}},"objects":[]}"#).unwrap();
        let values = apolline_scene::properties::Values::from([(
            "color".into(),
            serde_json::json!("0 1 0"),
        )]);
        assert_eq!(load(&folder, &values).unwrap().scene.clear, [0.0, 1.0, 0.0]);
        // An edit saved for a property the scene no longer has leaves its defaults alone.
        let stale =
            apolline_scene::properties::Values::from([("unknown".into(), serde_json::json!(true))]);
        assert_eq!(load(&folder, &stale).unwrap().scene.clear, [1.0, 0.0, 0.0]);
        std::fs::remove_dir_all(folder).unwrap();
    }
}
