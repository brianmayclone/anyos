use crate::block;

pub const HOTBAR_SLOTS: usize = 9;

pub struct Inventory {
    counts: [u16; block::BLOCK_COUNT],
    hotbar: [u8; HOTBAR_SLOTS],
    selected_slot: usize,
}

impl Inventory {
    pub fn new() -> Self {
        Self {
            counts: [0; block::BLOCK_COUNT],
            hotbar: [block::AIR; HOTBAR_SLOTS],
            selected_slot: 0,
        }
    }

    pub fn add_block(&mut self, block_id: u8) {
        if !block::is_collectible(block_id) {
            return;
        }
        let idx = block_id as usize;
        self.counts[idx] = self.counts[idx].saturating_add(1);
        if self.hotbar.contains(&block_id) {
            return;
        }
        if let Some(slot) = self.hotbar.iter().position(|&id| id == block::AIR) {
            self.hotbar[slot] = block_id;
            if self.selected_block().is_none() {
                self.selected_slot = slot;
            }
        }
    }

    pub fn selected_slot(&self) -> usize {
        self.selected_slot
    }

    pub fn set_selected_slot(&mut self, slot: usize) {
        self.selected_slot = slot.min(HOTBAR_SLOTS - 1);
        if self.selected_block().is_none() {
            self.select_next_filled(1);
        }
    }

    pub fn select_next_filled(&mut self, delta: i32) {
        let dir = if delta < 0 { -1 } else { 1 };
        for step in 0..HOTBAR_SLOTS {
            let idx = ((self.selected_slot as i32 + dir * step as i32)
                .rem_euclid(HOTBAR_SLOTS as i32)) as usize;
            if self.slot_count(idx) > 0 {
                self.selected_slot = idx;
                return;
            }
        }
    }

    pub fn selected_block(&self) -> Option<u8> {
        let block_id = self.hotbar[self.selected_slot];
        if self.counts[block_id as usize] > 0 {
            Some(block_id)
        } else {
            None
        }
    }

    pub fn selected_count(&self) -> u16 {
        self.slot_count(self.selected_slot)
    }

    pub fn slot_block(&self, slot: usize) -> Option<u8> {
        let block_id = self.hotbar[slot.min(HOTBAR_SLOTS - 1)];
        if self.counts[block_id as usize] > 0 {
            Some(block_id)
        } else {
            None
        }
    }

    pub fn slot_count(&self, slot: usize) -> u16 {
        let slot = slot.min(HOTBAR_SLOTS - 1);
        let block_id = self.hotbar[slot];
        self.counts[block_id as usize]
    }

    pub fn consume_selected(&mut self) -> bool {
        let Some(block_id) = self.selected_block() else {
            return false;
        };
        let idx = block_id as usize;
        if self.counts[idx] == 0 {
            return false;
        }
        self.counts[idx] -= 1;
        if self.counts[idx] == 0 {
            self.remove_from_hotbar(block_id);
        }
        true
    }

    fn remove_from_hotbar(&mut self, block_id: u8) {
        let mut compacted = [block::AIR; HOTBAR_SLOTS];
        let mut write = 0usize;
        for &entry in &self.hotbar {
            if entry != block_id && self.counts[entry as usize] > 0 {
                compacted[write] = entry;
                write += 1;
            }
        }
        self.hotbar = compacted;
        if self.selected_slot >= HOTBAR_SLOTS {
            self.selected_slot = HOTBAR_SLOTS - 1;
        }
        if self.selected_block().is_none() {
            self.selected_slot = 0;
            self.select_next_filled(1);
        }
    }

    pub fn counts_snapshot(&self) -> [u16; block::BLOCK_COUNT] {
        self.counts
    }

    pub fn hotbar_snapshot(&self) -> [u8; HOTBAR_SLOTS] {
        self.hotbar
    }

    pub fn restore_snapshot(
        &mut self,
        counts: [u16; block::BLOCK_COUNT],
        hotbar: [u8; HOTBAR_SLOTS],
        selected_slot: usize,
    ) {
        self.counts = counts;
        self.hotbar = hotbar;
        self.selected_slot = selected_slot.min(HOTBAR_SLOTS - 1);
        if self.selected_block().is_none() {
            self.select_next_filled(1);
        }
    }
}
