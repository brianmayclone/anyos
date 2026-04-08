pub const AIR: u8 = 0;
pub const STONE: u8 = 1;
pub const DIRT: u8 = 2;
pub const GRASS: u8 = 3;
pub const COBBLESTONE: u8 = 4;
pub const WOOD_PLANKS: u8 = 5;
pub const BEDROCK: u8 = 6;
pub const SAND: u8 = 7;
pub const GRAVEL: u8 = 8;
pub const WOOD_LOG: u8 = 9;
pub const LEAVES: u8 = 10;
pub const GLASS: u8 = 11;
pub const IRON_ORE: u8 = 12;
pub const COAL_ORE: u8 = 13;
pub const GOLD_ORE: u8 = 14;
pub const DIAMOND_ORE: u8 = 15;
pub const BRICK: u8 = 16;
pub const SNOW: u8 = 17;
pub const WATER: u8 = 18;
pub const CLAY: u8 = 19;
pub const TORCH: u8 = 20;

pub const BLOCK_COUNT: usize = 21;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ToolKind {
    Hand,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum MaterialKind {
    Fragile,
    Dirt,
    Wood,
    Stone,
    Metal,
    Plant,
    Utility,
    Liquid,
    Unbreakable,
}

pub const BLOCK_NAMES: [&str; BLOCK_COUNT] = [
    "Air",
    "Stone",
    "Dirt",
    "Grass",
    "Cobblestone",
    "Wood Planks",
    "Bedrock",
    "Sand",
    "Gravel",
    "Wood Log",
    "Leaves",
    "Glass",
    "Iron Ore",
    "Coal Ore",
    "Gold Ore",
    "Diamond Ore",
    "Brick",
    "Snow",
    "Water",
    "Clay",
    "Torch",
];

pub const BLOCK_SHORT_NAMES: [&str; BLOCK_COUNT] = [
    "--", "STN", "DIR", "GRS", "COB", "PLK", "BED", "SND", "GRV", "LOG", "LVS", "GLS", "IRO",
    "COL", "GLD", "DIA", "BRK", "SNW", "WTR", "CLY", "TRC",
];

pub fn is_solid(id: u8) -> bool {
    !matches!(id, AIR | WATER | TORCH)
}

pub fn is_transparent(id: u8) -> bool {
    matches!(id, AIR | WATER | GLASS | LEAVES | TORCH)
}

pub fn is_light_source(id: u8) -> bool {
    id == TORCH
}

pub fn is_collectible(id: u8) -> bool {
    !matches!(id, AIR | WATER | BEDROCK)
}

pub fn is_breakable(id: u8) -> bool {
    !matches!(id, AIR | WATER | BEDROCK)
}

pub fn material_kind(id: u8) -> MaterialKind {
    match id {
        AIR => MaterialKind::Utility,
        BEDROCK => MaterialKind::Unbreakable,
        WATER => MaterialKind::Liquid,
        LEAVES | SNOW => MaterialKind::Plant,
        DIRT | GRASS | SAND | GRAVEL | CLAY => MaterialKind::Dirt,
        WOOD_PLANKS | WOOD_LOG => MaterialKind::Wood,
        IRON_ORE | GOLD_ORE | DIAMOND_ORE | COAL_ORE => MaterialKind::Metal,
        TORCH | GLASS => MaterialKind::Fragile,
        STONE | COBBLESTONE | BRICK => MaterialKind::Stone,
        _ => MaterialKind::Stone,
    }
}

pub fn break_time_seconds(id: u8, tool: ToolKind) -> Option<f32> {
    if !is_breakable(id) {
        return None;
    }
    let time = match (material_kind(id), tool) {
        (MaterialKind::Fragile, ToolKind::Hand) => 0.18,
        (MaterialKind::Plant, ToolKind::Hand) => 0.25,
        (MaterialKind::Dirt, ToolKind::Hand) => 0.55,
        (MaterialKind::Wood, ToolKind::Hand) => 0.95,
        (MaterialKind::Stone, ToolKind::Hand) => 1.55,
        (MaterialKind::Metal, ToolKind::Hand) => 1.95,
        (MaterialKind::Utility, _) | (MaterialKind::Liquid, _) | (MaterialKind::Unbreakable, _) => return None,
    };
    Some(time)
}

/// Block face color (R, G, B) for HW rendering without textures.
pub fn block_color(id: u8, is_top: bool, is_bottom: bool) -> (f32, f32, f32) {
    match id {
        STONE       => (0.50, 0.50, 0.50),
        DIRT        => (0.55, 0.36, 0.24),
        GRASS if is_top    => (0.30, 0.65, 0.18),
        GRASS if is_bottom => (0.55, 0.36, 0.24),
        GRASS       => (0.40, 0.50, 0.22), // side: brown-green
        COBBLESTONE => (0.45, 0.45, 0.45),
        WOOD_PLANKS => (0.72, 0.56, 0.35),
        BEDROCK     => (0.20, 0.20, 0.20),
        SAND        => (0.85, 0.80, 0.55),
        GRAVEL      => (0.55, 0.52, 0.48),
        WOOD_LOG if is_top || is_bottom => (0.60, 0.48, 0.28),
        WOOD_LOG    => (0.40, 0.28, 0.15),
        LEAVES      => (0.20, 0.55, 0.12),
        GLASS       => (0.75, 0.85, 0.90),
        IRON_ORE    => (0.58, 0.55, 0.50),
        COAL_ORE    => (0.30, 0.30, 0.30),
        GOLD_ORE    => (0.70, 0.65, 0.25),
        DIAMOND_ORE => (0.35, 0.70, 0.75),
        BRICK       => (0.65, 0.32, 0.25),
        SNOW        => (0.92, 0.95, 0.98),
        WATER       => (0.20, 0.40, 0.75),
        CLAY        => (0.65, 0.60, 0.58),
        TORCH       => (0.95, 0.85, 0.40),
        _           => (0.80, 0.20, 0.80), // magenta = unknown
    }
}
