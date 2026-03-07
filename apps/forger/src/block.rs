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

pub fn is_solid(id: u8) -> bool {
    !matches!(id, AIR | WATER | TORCH)
}

pub fn is_transparent(id: u8) -> bool {
    matches!(id, AIR | WATER | GLASS | LEAVES | TORCH)
}

pub fn is_light_source(id: u8) -> bool {
    id == TORCH
}
