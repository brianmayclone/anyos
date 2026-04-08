use alloc::string::String;
use anyos_std::json::Value;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum GraphicsQuality {
    Fast,
    Balanced,
    Fancy,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ShadowQuality {
    Soft,
    Balanced,
    Crisp,
}

#[derive(Clone)]
pub struct GameSettings {
    pub graphics_quality: GraphicsQuality,
    pub shadows_enabled: bool,
    pub shadow_quality: ShadowQuality,
}

impl GameSettings {
    pub fn defaults() -> Self {
        Self {
            graphics_quality: GraphicsQuality::Balanced,
            shadows_enabled: false,
            shadow_quality: ShadowQuality::Balanced,
        }
    }

    pub fn load() -> Self {
        let path = settings_path();
        let Ok(text) = anyos_std::fs::read_to_string(&path) else {
            let settings = Self::defaults();
            settings.save();
            return settings;
        };
        let Ok(json) = Value::parse(&text) else {
            return Self::defaults();
        };

        Self {
            graphics_quality: match json["graphics_quality"].as_str().unwrap_or("balanced") {
                "fast" => GraphicsQuality::Fast,
                "fancy" => GraphicsQuality::Fancy,
                _ => GraphicsQuality::Balanced,
            },
            shadows_enabled: json["shadows_enabled"].as_bool().unwrap_or(false),
            shadow_quality: match json["shadow_quality"].as_str().unwrap_or("balanced") {
                "soft" => ShadowQuality::Soft,
                "crisp" => ShadowQuality::Crisp,
                _ => ShadowQuality::Balanced,
            },
        }
    }

    pub fn save(&self) {
        crate::save::mkdir_p(&crate::save::data_root());
        let mut root = Value::new_object();
        root.set("graphics_quality", self.graphics_quality_key().into());
        root.set("shadows_enabled", self.shadows_enabled.into());
        root.set("shadow_quality", self.shadow_quality_key().into());
        let json = root.to_json_string_pretty();
        let _ = anyos_std::fs::write_bytes(&settings_path(), json.as_bytes());
    }

    pub fn graphics_quality_index(&self) -> u32 {
        match self.graphics_quality {
            GraphicsQuality::Fast => 0,
            GraphicsQuality::Balanced => 1,
            GraphicsQuality::Fancy => 2,
        }
    }

    pub fn shadow_quality_index(&self) -> u32 {
        match self.shadow_quality {
            ShadowQuality::Soft => 0,
            ShadowQuality::Balanced => 1,
            ShadowQuality::Crisp => 2,
        }
    }

    pub fn graphics_quality_label(&self) -> &'static str {
        match self.graphics_quality {
            GraphicsQuality::Fast => "Schnell",
            GraphicsQuality::Balanced => "Normal",
            GraphicsQuality::Fancy => "Hoch",
        }
    }

    pub fn shadow_quality_label(&self) -> &'static str {
        match self.shadow_quality {
            ShadowQuality::Soft => "Weich",
            ShadowQuality::Balanced => "Normal",
            ShadowQuality::Crisp => "Scharf",
        }
    }

    pub fn render_divisor(&self) -> u32 {
        match self.graphics_quality {
            GraphicsQuality::Fast => 3,
            GraphicsQuality::Balanced => 2,
            GraphicsQuality::Fancy => 1,
        }
    }

    pub fn fog_distance(&self) -> f32 {
        match self.graphics_quality {
            GraphicsQuality::Fast => 26.0,
            GraphicsQuality::Balanced => 38.0,
            GraphicsQuality::Fancy => 54.0,
        }
    }

    pub fn shadow_strength(&self) -> f32 {
        match self.shadow_quality {
            ShadowQuality::Soft => 0.58,
            ShadowQuality::Balanced => 0.72,
            ShadowQuality::Crisp => 0.86,
        }
    }

    pub fn shadow_softness_scale(&self) -> f32 {
        match self.shadow_quality {
            ShadowQuality::Soft => 2.25,
            ShadowQuality::Balanced => 1.5,
            ShadowQuality::Crisp => 0.95,
        }
    }

    pub fn set_graphics_quality_from_index(&mut self, index: u32) {
        self.graphics_quality = match index {
            0 => GraphicsQuality::Fast,
            2 => GraphicsQuality::Fancy,
            _ => GraphicsQuality::Balanced,
        };
    }

    pub fn set_shadow_quality_from_index(&mut self, index: u32) {
        self.shadow_quality = match index {
            0 => ShadowQuality::Soft,
            2 => ShadowQuality::Crisp,
            _ => ShadowQuality::Balanced,
        };
    }

    fn graphics_quality_key(&self) -> &'static str {
        match self.graphics_quality {
            GraphicsQuality::Fast => "fast",
            GraphicsQuality::Balanced => "balanced",
            GraphicsQuality::Fancy => "fancy",
        }
    }

    fn shadow_quality_key(&self) -> &'static str {
        match self.shadow_quality {
            ShadowQuality::Soft => "soft",
            ShadowQuality::Balanced => "balanced",
            ShadowQuality::Crisp => "crisp",
        }
    }
}

fn settings_path() -> String {
    alloc::format!("{}/settings.json", crate::save::data_root())
}
