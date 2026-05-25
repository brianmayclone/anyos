use super::{exit_reason, VmExitInfo};

pub(super) const IOAPIC_MMIO_BASE: u64 = 0xfec0_0000;
const IOAPIC_MMIO_SIZE: u64 = 0x20;
pub(super) const LAPIC_MMIO_BASE: u64 = 0xfee0_0000;
const LAPIC_MMIO_SIZE: u64 = 0x1000;

const LAPIC_ID: u32 = 0x20;
const LAPIC_VERSION: u32 = 0x30;
const LAPIC_TPR: u32 = 0x80;
const LAPIC_EOI: u32 = 0xb0;
const LAPIC_SVR: u32 = 0xf0;
const LAPIC_ICR_LOW: u32 = 0x300;
const LAPIC_ICR_HIGH: u32 = 0x310;
const LAPIC_LVT_TIMER: u32 = 0x320;
const LAPIC_LVT_LINT0: u32 = 0x350;
const LAPIC_LVT_LINT1: u32 = 0x360;
const LAPIC_LVT_ERROR: u32 = 0x370;
const LAPIC_TIMER_INITIAL_COUNT: u32 = 0x380;
const LAPIC_TIMER_CURRENT_COUNT: u32 = 0x390;
const LAPIC_TIMER_DIVIDE_CONFIG: u32 = 0x3e0;
const LAPIC_LVT_MASKED: u32 = 1 << 16;
const LAPIC_LVT_TIMER_PERIODIC: u32 = 1 << 17;
const LAPIC_TIMER_HZ: u64 = 100_000_000;

const IOAPIC_REG_ID: u8 = 0x00;
const IOAPIC_REG_VERSION: u8 = 0x01;
const IOAPIC_REG_ARBITRATION_ID: u8 = 0x02;
const IOAPIC_REDIR_BASE: u8 = 0x10;
const IOAPIC_PIN_COUNT: usize = 24;
const IOAPIC_REDIR_MASKED: u32 = 1 << 16;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct ApicMmioAction {
    pub read_value: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ApicState {
    lapic: LocalApicState,
    ioapic: IoApicState,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct LocalApicState {
    tpr: u32,
    svr: u32,
    icr_low: u32,
    icr_high: u32,
    lvt_timer: u32,
    lvt_lint0: u32,
    lvt_lint1: u32,
    lvt_error: u32,
    timer_initial_count: u32,
    timer_divide_config: u32,
    timer_started_ms: u32,
    timer_next_irq_ms: u32,
    timer_period_ms: u32,
    timer_pending_vector: Option<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct IoApicState {
    select: u8,
    id: u8,
    redir: [IoApicRedirectionEntry; IOAPIC_PIN_COUNT],
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
struct IoApicRedirectionEntry {
    low: u32,
    high: u32,
}

impl Default for ApicState {
    fn default() -> Self {
        Self {
            lapic: LocalApicState::default(),
            ioapic: IoApicState::default(),
        }
    }
}

impl Default for LocalApicState {
    fn default() -> Self {
        Self {
            tpr: 0,
            svr: 0xff,
            icr_low: 0,
            icr_high: 0,
            lvt_timer: LAPIC_LVT_MASKED,
            lvt_lint0: LAPIC_LVT_MASKED,
            lvt_lint1: LAPIC_LVT_MASKED,
            lvt_error: LAPIC_LVT_MASKED,
            timer_initial_count: 0,
            timer_divide_config: 0,
            timer_started_ms: 0,
            timer_next_irq_ms: 0,
            timer_period_ms: 1,
            timer_pending_vector: None,
        }
    }
}

impl Default for IoApicState {
    fn default() -> Self {
        Self {
            select: 0,
            id: 0,
            redir: [IoApicRedirectionEntry::masked(); IOAPIC_PIN_COUNT],
        }
    }
}

impl IoApicRedirectionEntry {
    const fn masked() -> Self {
        Self {
            low: IOAPIC_REDIR_MASKED,
            high: 0,
        }
    }
}

impl ApicState {
    pub(super) fn irq_vector(&self, irq: u8) -> Option<u8> {
        self.ioapic.irq_vector(irq)
    }

    pub(super) fn pending_local_irq(&mut self) -> Option<u8> {
        self.lapic.refresh(anyos_std::sys::uptime_ms());
        self.lapic.pending_irq()
    }

    pub(super) fn ack_local_irq(&mut self, vector: u8) {
        self.lapic.ack_irq(vector);
    }
}

pub(super) fn apic_mmio_action(state: &mut ApicState, exit: &VmExitInfo) -> Option<ApicMmioAction> {
    if exit.reason != exit_reason::EPT_VIOLATION {
        return None;
    }

    if is_lapic_mmio(exit.guest_phys_addr) {
        return Some(lapic_mmio_action(
            &mut state.lapic,
            exit.guest_phys_addr - LAPIC_MMIO_BASE,
            exit,
        ));
    }

    if is_ioapic_mmio(exit.guest_phys_addr) {
        return Some(ioapic_mmio_action(
            &mut state.ioapic,
            exit.guest_phys_addr - IOAPIC_MMIO_BASE,
            exit,
        ));
    }

    None
}

pub(super) fn is_apic_mmio(gpa: u64) -> bool {
    is_lapic_mmio(gpa) || is_ioapic_mmio(gpa)
}

fn lapic_mmio_action(state: &mut LocalApicState, offset: u64, exit: &VmExitInfo) -> ApicMmioAction {
    let reg = (offset as u32) & !0x0f;
    if exit.is_read != 0 {
        return ApicMmioAction {
            read_value: Some(lapic_read(state, reg)),
        };
    }

    lapic_write(state, reg, exit.io_data as u32);
    ApicMmioAction::default()
}

fn lapic_read(state: &LocalApicState, reg: u32) -> u32 {
    match reg {
        LAPIC_ID => 0,
        LAPIC_VERSION => 0x14 | (5 << 16),
        LAPIC_TPR => state.tpr,
        LAPIC_EOI => 0,
        LAPIC_SVR => state.svr,
        LAPIC_ICR_LOW => state.icr_low,
        LAPIC_ICR_HIGH => state.icr_high,
        LAPIC_LVT_TIMER => state.lvt_timer,
        LAPIC_LVT_LINT0 => state.lvt_lint0,
        LAPIC_LVT_LINT1 => state.lvt_lint1,
        LAPIC_LVT_ERROR => state.lvt_error,
        LAPIC_TIMER_INITIAL_COUNT => state.timer_initial_count,
        LAPIC_TIMER_CURRENT_COUNT => state.current_timer_count(anyos_std::sys::uptime_ms()),
        LAPIC_TIMER_DIVIDE_CONFIG => state.timer_divide_config,
        _ => 0,
    }
}

fn lapic_write(state: &mut LocalApicState, reg: u32, value: u32) {
    match reg {
        LAPIC_TPR => state.tpr = value,
        LAPIC_EOI => {}
        LAPIC_SVR => state.svr = value,
        LAPIC_ICR_LOW => state.icr_low = value,
        LAPIC_ICR_HIGH => state.icr_high = value,
        LAPIC_LVT_TIMER => {
            state.lvt_timer = value;
            state.rearm_timer(anyos_std::sys::uptime_ms());
        }
        LAPIC_LVT_LINT0 => state.lvt_lint0 = value,
        LAPIC_LVT_LINT1 => state.lvt_lint1 = value,
        LAPIC_LVT_ERROR => state.lvt_error = value,
        LAPIC_TIMER_INITIAL_COUNT => state.program_timer_count(value, anyos_std::sys::uptime_ms()),
        LAPIC_TIMER_DIVIDE_CONFIG => {
            state.timer_divide_config = value & 0x0f;
            state.rearm_timer(anyos_std::sys::uptime_ms());
        }
        _ => {}
    }
}

impl LocalApicState {
    fn program_timer_count(&mut self, value: u32, now_ms: u32) {
        self.timer_initial_count = value;
        self.rearm_timer(now_ms);
    }

    fn rearm_timer(&mut self, now_ms: u32) {
        self.timer_pending_vector = None;
        self.timer_started_ms = now_ms;
        if self.timer_initial_count == 0 || self.timer_vector().is_none() {
            return;
        }
        self.timer_period_ms = self.timer_period_ms().max(1);
        self.timer_next_irq_ms = now_ms.wrapping_add(self.timer_period_ms);
    }

    fn refresh(&mut self, now_ms: u32) {
        if self.timer_pending_vector.is_some()
            || self.timer_initial_count == 0
            || !time_due(now_ms, self.timer_next_irq_ms)
        {
            return;
        }

        let Some(vector) = self.timer_vector() else {
            return;
        };
        self.timer_pending_vector = Some(vector);
        if self.lvt_timer & LAPIC_LVT_TIMER_PERIODIC != 0 {
            let period = self.timer_period_ms.max(1);
            self.timer_next_irq_ms = self.timer_next_irq_ms.wrapping_add(period);
            for _ in 0..16 {
                if !time_due(now_ms, self.timer_next_irq_ms) {
                    break;
                }
                self.timer_next_irq_ms = self.timer_next_irq_ms.wrapping_add(period);
            }
        } else {
            self.timer_initial_count = 0;
        }
    }

    fn pending_irq(&self) -> Option<u8> {
        self.timer_pending_vector
    }

    fn ack_irq(&mut self, vector: u8) {
        if self.timer_pending_vector == Some(vector) {
            self.timer_pending_vector = None;
        }
    }

    fn timer_vector(&self) -> Option<u8> {
        if self.lvt_timer & LAPIC_LVT_MASKED != 0 {
            return None;
        }
        let vector = (self.lvt_timer & 0xff) as u8;
        if vector == 0 {
            None
        } else {
            Some(vector)
        }
    }

    fn timer_period_ms(&self) -> u32 {
        let divisor = lapic_timer_divisor(self.timer_divide_config) as u64;
        let ticks_per_ms = (LAPIC_TIMER_HZ / divisor / 1000).max(1);
        ((self.timer_initial_count as u64 + ticks_per_ms - 1) / ticks_per_ms) as u32
    }

    fn current_timer_count(&self, now_ms: u32) -> u32 {
        if self.timer_initial_count == 0 {
            return 0;
        }
        let divisor = lapic_timer_divisor(self.timer_divide_config) as u64;
        let ticks_per_ms = (LAPIC_TIMER_HZ / divisor / 1000).max(1);
        let elapsed = now_ms.wrapping_sub(self.timer_started_ms) as u64;
        let elapsed_ticks = elapsed.saturating_mul(ticks_per_ms);
        if self.lvt_timer & LAPIC_LVT_TIMER_PERIODIC != 0 {
            let initial = self.timer_initial_count as u64;
            let remaining = initial - (elapsed_ticks % initial);
            remaining.min(u32::MAX as u64) as u32
        } else {
            (self.timer_initial_count as u64)
                .saturating_sub(elapsed_ticks)
                .min(u32::MAX as u64) as u32
        }
    }
}

fn lapic_timer_divisor(config: u32) -> u32 {
    match config & 0x0f {
        0b1011 => 1,
        0b0000 => 2,
        0b0001 => 4,
        0b0010 => 8,
        0b0011 => 16,
        0b1000 => 32,
        0b1001 => 64,
        0b1010 => 128,
        _ => 2,
    }
}

fn time_due(now: u32, deadline: u32) -> bool {
    now.wrapping_sub(deadline) <= 0x7fff_ffff
}

fn ioapic_mmio_action(state: &mut IoApicState, offset: u64, exit: &VmExitInfo) -> ApicMmioAction {
    match offset {
        0x00 => {
            if exit.is_read != 0 {
                ApicMmioAction {
                    read_value: Some(state.select as u32),
                }
            } else {
                state.select = (exit.io_data & 0xff) as u8;
                ApicMmioAction::default()
            }
        }
        0x10 => {
            if exit.is_read != 0 {
                ApicMmioAction {
                    read_value: Some(state.read_selected()),
                }
            } else {
                state.write_selected(exit.io_data as u32);
                ApicMmioAction::default()
            }
        }
        _ => ApicMmioAction::default(),
    }
}

impl IoApicState {
    fn read_selected(&self) -> u32 {
        match self.select {
            IOAPIC_REG_ID => (self.id as u32) << 24,
            IOAPIC_REG_VERSION => 0x11 | (((IOAPIC_PIN_COUNT as u32) - 1) << 16),
            IOAPIC_REG_ARBITRATION_ID => (self.id as u32) << 24,
            reg => self
                .redir_index(reg)
                .map(|(index, high)| {
                    if high {
                        self.redir[index].high
                    } else {
                        self.redir[index].low
                    }
                })
                .unwrap_or(0),
        }
    }

    fn write_selected(&mut self, value: u32) {
        match self.select {
            IOAPIC_REG_ID => self.id = ((value >> 24) & 0x0f) as u8,
            reg => {
                if let Some((index, high)) = self.redir_index(reg) {
                    if high {
                        self.redir[index].high = value;
                    } else {
                        self.redir[index].low = value;
                    }
                }
            }
        }
    }

    fn irq_vector(&self, irq: u8) -> Option<u8> {
        let entry = self.redir.get(irq as usize)?;
        if entry.low & IOAPIC_REDIR_MASKED != 0 {
            return None;
        }
        let vector = (entry.low & 0xff) as u8;
        if vector == 0 {
            None
        } else {
            Some(vector)
        }
    }

    fn redir_index(&self, reg: u8) -> Option<(usize, bool)> {
        let relative = reg.checked_sub(IOAPIC_REDIR_BASE)?;
        let index = (relative / 2) as usize;
        if index >= IOAPIC_PIN_COUNT {
            return None;
        }
        Some((index, relative & 1 != 0))
    }
}

fn is_lapic_mmio(gpa: u64) -> bool {
    (LAPIC_MMIO_BASE..LAPIC_MMIO_BASE + LAPIC_MMIO_SIZE).contains(&gpa)
}

fn is_ioapic_mmio(gpa: u64) -> bool {
    (IOAPIC_MMIO_BASE..IOAPIC_MMIO_BASE + IOAPIC_MMIO_SIZE).contains(&gpa)
}

#[cfg(test)]
mod tests {
    use super::{apic_mmio_action, ApicState, IOAPIC_MMIO_BASE, LAPIC_MMIO_BASE};
    use crate::vm::{exit_reason, VmExitInfo};

    fn mmio_read(state: &mut ApicState, gpa: u64) -> u32 {
        apic_mmio_action(
            state,
            &VmExitInfo {
                reason: exit_reason::EPT_VIOLATION,
                is_read: 1,
                access_size: 4,
                guest_phys_addr: gpa,
                ..VmExitInfo::default()
            },
        )
        .and_then(|action| action.read_value)
        .unwrap()
    }

    fn mmio_write(state: &mut ApicState, gpa: u64, value: u32) {
        let _ = apic_mmio_action(
            state,
            &VmExitInfo {
                reason: exit_reason::EPT_VIOLATION,
                is_read: 0,
                access_size: 4,
                guest_phys_addr: gpa,
                io_data: value as u64,
                ..VmExitInfo::default()
            },
        );
    }

    #[test]
    fn ioapic_routes_unmasked_irq_vectors() {
        let mut state = ApicState::default();
        assert_eq!(state.irq_vector(11), None);

        mmio_write(&mut state, IOAPIC_MMIO_BASE, 0x10 + 11 * 2);
        mmio_write(&mut state, IOAPIC_MMIO_BASE + 0x10, 0x2b);

        assert_eq!(state.irq_vector(11), Some(0x2b));

        mmio_write(&mut state, IOAPIC_MMIO_BASE, 0x10 + 11 * 2);
        assert_eq!(mmio_read(&mut state, IOAPIC_MMIO_BASE + 0x10), 0x2b);
    }

    #[test]
    fn lapic_exposes_version_and_eoi() {
        let mut state = ApicState::default();
        assert_eq!(mmio_read(&mut state, LAPIC_MMIO_BASE + 0x30) & 0xff, 0x14);
        mmio_write(&mut state, LAPIC_MMIO_BASE + 0xb0, 0);
    }

    #[test]
    fn lapic_periodic_timer_raises_local_vector() {
        let mut state = ApicState::default();
        state.lapic.lvt_timer = super::LAPIC_LVT_TIMER_PERIODIC | 0xec;
        state.lapic.timer_divide_config = 0;
        state.lapic.program_timer_count(100_000, 1000);

        state.lapic.refresh(1001);
        assert_eq!(state.lapic.pending_irq(), None);
        state.lapic.refresh(1002);
        assert_eq!(state.lapic.pending_irq(), Some(0xec));
        state.ack_local_irq(0xec);
        assert_eq!(state.lapic.pending_irq(), None);
    }
}
