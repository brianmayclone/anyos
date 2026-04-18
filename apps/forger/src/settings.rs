use alloc::string::String;
use anyos_std::json::Value;
use libconf_schema::{default_bool, default_string, manifest, RegistryScope, ServiceSchema};

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

const FORGER_SETTINGS_DEFAULTS: &[libconf_schema::DefaultEntry<'static>] = &[
    default_string("config/graphics_quality", "balanced"),
    default_bool("config/shadows_enabled", false),
    default_string("config/shadow_quality", "balanced"),
];
const FORGER_SETTINGS_MIGRATIONS: &[libconf_schema::MigrationStep<'static>] = &[];
const FORGER_SETTINGS_MANIFEST: libconf_schema::RegistryManifest<'static> = manifest(
    "apps/forger",
    RegistryScope::User,
    1,
    &["config"],
    FORGER_SETTINGS_DEFAULTS,
    FORGER_SETTINGS_MIGRATIONS,
);

fn schema() -> ServiceSchema<'static> {
    ServiceSchema::new("forger", &FORGER_SETTINGS_MANIFEST)
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
        let _ = schema().register();
        if let Some(settings) = load_from_confd() {
            return settings;
        }
        let settings = Self::defaults();
        settings.save();
        settings
    }

    pub fn save(&self) {
        let _ = schema().register();
        let _ = schema().write_string("config/graphics_quality", self.graphics_quality_key());
        let _ = schema().write_bool("config/shadows_enabled", self.shadows_enabled);
        let _ = schema().write_string("config/shadow_quality", self.shadow_quality_key());
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

fn load_from_confd() -> Option<GameSettings> {
    let graphics_quality = match schema().read_string("config/graphics_quality")?.as_str() {
        "fast" => GraphicsQuality::Fast,
        "fancy" => GraphicsQuality::Fancy,
        _ => GraphicsQuality::Balanced,
    };
    let shadows_enabled = schema().read_bool("config/shadows_enabled").unwrap_or(false);
    let shadow_quality = match schema().read_string("config/shadow_quality")?.as_str() {
        "soft" => ShadowQuality::Soft,
        "crisp" => ShadowQuality::Crisp,
        _ => ShadowQuality::Balanced,
    };
    Some(GameSettings {
        graphics_quality,
        shadows_enabled,
        shadow_quality,
    })
}
