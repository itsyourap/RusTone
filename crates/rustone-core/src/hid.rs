//! Telephony headsets (USB HID, usage page 0x0B):
//! where a headset's hook switch, mute and redial buttons sit in its input
//! reports, and its off-hook, ring and mute lights in its output reports,
//! read from the HID report descriptor. Windows' HidP_* functions could do this, but
//! this is a small descriptor parser so it works anywhere.

use std::collections::HashMap;

pub const PAGE_LED: u16 = 0x08;
pub const PAGE_TELEPHONY: u16 = 0x0B;

pub const HOOK_SWITCH: u16 = 0x20;
pub const FLASH: u16 = 0x21;
pub const REDIAL: u16 = 0x24;
pub const PHONE_MUTE: u16 = 0x2F;

pub const LED_MUTE: u16 = 0x09;
pub const LED_OFF_HOOK: u16 = 0x17;
pub const LED_RING: u16 = 0x18;

/// Where one usage sits in a report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Field {
    pub report_id: u8,
    /// First bit after the report id byte.
    pub bit: u32,
    pub size: u32,
    /// A button that reports presses (toggle), not a state.
    pub relative: bool,
}

/// A selector array: each slot holds the index of a pressed usage.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ArrayField {
    report_id: u8,
    bit: u32,
    size: u32,
    count: u32,
    logical_min: i32,
    /// The usages, in index order.
    usages: Vec<(u16, u16)>,
}

/// What RusTone uses of a telephony headset.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Layout {
    /// Whether reports start with a report id byte.
    pub numbered: bool,
    pub hook_switch: Option<Field>,
    pub mute: Option<Field>,
    pub redial: Option<Field>,
    pub flash: Option<Field>,
    pub led_off_hook: Option<Field>,
    pub led_ring: Option<Field>,
    pub led_mute: Option<Field>,
    arrays: Vec<ArrayField>,
    /// Output report sizes in bytes (without the id), by report id.
    output_bytes: HashMap<u8, usize>,
}

impl Layout {
    /// Whether the headset has a hook switch, the part calls need.
    pub fn is_usable(&self) -> bool {
        self.hook_switch.is_some() || self.arrays.iter().any(|a| a.usages.contains(&(PAGE_TELEPHONY, HOOK_SWITCH)))
    }

    /// Whether `usage` (telephony page) is on in an input report as read
    /// (with the id byte when numbered). `None` when the report isn't the
    /// one holding it.
    pub fn input(&self, usage: u16, report: &[u8]) -> Option<bool> {
        let (id, data) = self.split(report);
        let field = match usage {
            HOOK_SWITCH => self.hook_switch,
            PHONE_MUTE => self.mute,
            REDIAL => self.redial,
            FLASH => self.flash,
            _ => None,
        };
        if let Some(f) = field.filter(|f| f.report_id == id) {
            return Some(bits(data, f.bit, f.size) != 0);
        }
        let array = self.arrays.iter().find(|a| a.report_id == id && a.usages.contains(&(PAGE_TELEPHONY, usage)))?;
        let index = array.usages.iter().position(|u| *u == (PAGE_TELEPHONY, usage))? as i64 + i64::from(array.logical_min);
        Some((0..array.count).any(|slot| i64::from(bits(data, array.bit + slot * array.size, array.size)) == index))
    }

    /// Output reports (id byte first, as hidapi writes them) lighting the
    /// LEDs this way. One per report id holding any of them.
    pub fn leds(&self, off_hook: bool, ring: bool, mute: bool) -> Vec<Vec<u8>> {
        let mut reports: HashMap<u8, Vec<u8>> = HashMap::new();
        for (field, on) in [(self.led_off_hook, off_hook), (self.led_ring, ring), (self.led_mute, mute)] {
            let Some(f) = field else { continue };
            let size = self.output_bytes.get(&f.report_id).copied().unwrap_or(1);
            let report = reports.entry(f.report_id).or_insert_with(|| {
                let mut r = vec![0u8; size + 1];
                r[0] = f.report_id;
                r
            });
            if on {
                set_bits(&mut report[1..], f.bit, f.size, 1);
            }
        }
        let mut out: Vec<(u8, Vec<u8>)> = reports.into_iter().collect();
        out.sort_by_key(|(id, _)| *id);
        out.into_iter().map(|(_, r)| r).collect()
    }

    fn split<'a>(&self, report: &'a [u8]) -> (u8, &'a [u8]) {
        if self.numbered {
            match report.split_first() {
                Some((id, data)) => (*id, data),
                None => (0, &[]),
            }
        } else {
            (0, report)
        }
    }
}

fn bits(data: &[u8], bit: u32, size: u32) -> u32 {
    let mut value = 0;
    for i in 0..size.min(32) {
        let at = bit + i;
        let byte = data.get((at / 8) as usize).copied().unwrap_or(0);
        if byte >> (at % 8) & 1 == 1 {
            value |= 1 << i;
        }
    }
    value
}

fn set_bits(data: &mut [u8], bit: u32, size: u32, value: u32) {
    for i in 0..size.min(32) {
        let at = bit + i;
        if let Some(byte) = data.get_mut((at / 8) as usize) {
            if value >> i & 1 == 1 {
                *byte |= 1 << (at % 8);
            } else {
                *byte &= !(1 << (at % 8));
            }
        }
    }
}

/// Reads a report descriptor.
pub fn parse(descriptor: &[u8]) -> Layout {
    let mut layout = Layout::default();
    // Globals.
    let (mut page, mut size, mut count, mut report_id, mut logical_min) = (0u16, 0u32, 0u32, 0u8, 0i32);
    // Locals, cleared after each main item.
    let mut usages: Vec<(u16, u16)> = Vec::new();
    let (mut usage_min, mut usage_max): (Option<(u16, u16)>, Option<u16>) = (None, None);
    // Next free bit per (main kind, report id).
    let mut position: HashMap<(u8, u8), u32> = HashMap::new();
    let mut i = 0;
    while i < descriptor.len() {
        let prefix = descriptor[i];
        if prefix == 0xFE {
            // Long item: size, tag, data.
            let len = descriptor.get(i + 1).copied().unwrap_or(0) as usize;
            i += 3 + len;
            continue;
        }
        let len = match prefix & 0x03 {
            3 => 4,
            n => n as usize,
        };
        let data = descriptor.get(i + 1..i + 1 + len).unwrap_or(&[]);
        let unsigned = data.iter().rev().fold(0u32, |v, b| v << 8 | u32::from(*b));
        let signed = match len {
            1 => i32::from(unsigned as u8 as i8),
            2 => i32::from(unsigned as u16 as i16),
            _ => unsigned as i32,
        };
        let usage = |value: u32, page: u16| if len == 4 { ((value >> 16) as u16, value as u16) } else { (page, value as u16) };
        match (prefix >> 2) & 0x03 {
            // Main.
            0 => {
                let tag = prefix >> 4;
                if matches!(tag, 0x8 | 0x9 | 0xB) {
                    let kind = tag;
                    let at = position.entry((kind, report_id)).or_insert(0);
                    let constant = unsigned & 0x01 != 0;
                    let variable = unsigned & 0x02 != 0;
                    let relative = unsigned & 0x04 != 0;
                    let mut list = usages.clone();
                    if let (Some((p, min)), Some(max)) = (usage_min, usage_max) {
                        list.extend((min..=max).map(|u| (p, u)));
                    }
                    if !constant && !list.is_empty() {
                        if variable {
                            for slot in 0..count {
                                let u = list.get(slot as usize).or(list.last()).copied().unwrap_or((page, 0));
                                let field = Field { report_id, bit: *at + slot * size, size, relative };
                                place(&mut layout, kind, u, field);
                            }
                        } else if kind == 0x8 {
                            layout.arrays.push(ArrayField { report_id, bit: *at, size, count, logical_min, usages: list });
                        }
                    }
                    *at += size * count;
                    if kind == 0x9 {
                        let bytes = (*at).div_ceil(8) as usize;
                        layout.output_bytes.insert(report_id, bytes);
                    }
                }
                usages.clear();
                usage_min = None;
                usage_max = None;
            }
            // Global.
            1 => match prefix >> 4 {
                0x0 => page = unsigned as u16,
                0x1 => logical_min = signed,
                0x7 => size = unsigned,
                0x8 => {
                    report_id = unsigned as u8;
                    layout.numbered = true;
                }
                0x9 => count = unsigned,
                _ => {}
            },
            // Local.
            2 => match prefix >> 4 {
                0x0 => usages.push(usage(unsigned, page)),
                0x1 => usage_min = Some(usage(unsigned, page)),
                0x2 => usage_max = Some(unsigned as u16),
                _ => {}
            },
            _ => {}
        }
        i += 1 + len;
    }
    layout
}

fn place(layout: &mut Layout, kind: u8, (page, usage): (u16, u16), field: Field) {
    let slot = match (kind, page, usage) {
        (0x8, PAGE_TELEPHONY, HOOK_SWITCH) => &mut layout.hook_switch,
        (0x8, PAGE_TELEPHONY, PHONE_MUTE) => &mut layout.mute,
        (0x8, PAGE_TELEPHONY, REDIAL) => &mut layout.redial,
        (0x8, PAGE_TELEPHONY, FLASH) => &mut layout.flash,
        (0x9, PAGE_LED, LED_OFF_HOOK) => &mut layout.led_off_hook,
        (0x9, PAGE_LED, LED_RING) => &mut layout.led_ring,
        (0x9, PAGE_LED, LED_MUTE) => &mut layout.led_mute,
        _ => return,
    };
    if slot.is_none() {
        *slot = Some(field);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A typical headset's telephony collection: report 2 has hook switch
    /// and phone mute (absolute), redial (relative) and padding, an array of
    /// two buttons in report 3; report 2 output has the LEDs.
    const HEADSET: &[u8] = &[
        0x05, 0x0B, // Usage Page (Telephony)
        0x09, 0x05, // Usage (Headset)
        0xA1, 0x01, // Collection (Application)
        0x85, 0x02, //   Report ID (2)
        0x15, 0x00, 0x25, 0x01, // Logical 0..1
        0x75, 0x01, 0x95, 0x02, //   Report Size 1, Count 2
        0x09, 0x20, 0x09, 0x2F, //   Usage (Hook Switch), Usage (Phone Mute)
        0x81, 0x02, //   Input (Data, Variable, Absolute)
        0x95, 0x01, 0x09, 0x24, //   Count 1, Usage (Redial)
        0x81, 0x06, //   Input (Data, Variable, Relative)
        0x95, 0x05, 0x81, 0x01, //   Count 5, Input (Constant): padding
        0x05, 0x08, //   Usage Page (LED)
        0x95, 0x03, //   Count 3
        0x09, 0x17, 0x09, 0x18, 0x09, 0x09, // Off-Hook, Ring, Mute
        0x91, 0x02, //   Output (Data, Variable, Absolute)
        0x95, 0x05, 0x91, 0x01, //   Output padding
        0x05, 0x0B, //   Usage Page (Telephony)
        0x85, 0x03, //   Report ID (3)
        0x15, 0x01, 0x25, 0x02, //   Logical 1..2
        0x75, 0x02, 0x95, 0x01, //   Size 2, Count 1
        0x09, 0x21, 0x09, 0x24, //   Usage (Flash), Usage (Redial)
        0x81, 0x00, //   Input (Data, Array)
        0x95, 0x01, 0x75, 0x06, 0x81, 0x01, // padding
        0xC0, // End Collection
    ];

    #[test]
    fn a_headset_descriptor_is_read() {
        let layout = parse(HEADSET);
        assert!(layout.numbered && layout.is_usable());
        assert_eq!(layout.hook_switch, Some(Field { report_id: 2, bit: 0, size: 1, relative: false }));
        assert_eq!(layout.mute.map(|f| f.bit), Some(1));
        assert_eq!(layout.redial.map(|f| (f.bit, f.relative)), Some((2, true)));
        assert_eq!(layout.led_ring.map(|f| (f.report_id, f.bit)), Some((2, 1)));

        // Hook switch on, mute off.
        assert_eq!(layout.input(HOOK_SWITCH, &[2, 0b0000_0001]), Some(true));
        assert_eq!(layout.input(PHONE_MUTE, &[2, 0b0000_0001]), Some(false));
        assert_eq!(layout.input(PHONE_MUTE, &[2, 0b0000_0010]), Some(true));
        // Another report doesn't say.
        assert_eq!(layout.input(HOOK_SWITCH, &[3, 0xFF]), None);
        // The array in report 3: index 1 is Flash.
        assert_eq!(layout.input(FLASH, &[3, 0b01]), Some(true));
        assert_eq!(layout.input(FLASH, &[3, 0b00]), Some(false));
    }

    #[test]
    fn leds_go_into_one_output_report() {
        let layout = parse(HEADSET);
        assert_eq!(layout.leds(true, false, true), vec![vec![2, 0b101]]);
        assert_eq!(layout.leds(false, false, false), vec![vec![2, 0]]);
    }

    #[test]
    fn a_keyboard_is_not_a_headset() {
        let keyboard = [0x05, 0x01, 0x09, 0x06, 0xA1, 0x01, 0x05, 0x07, 0x19, 0xE0, 0x29, 0xE7, 0x15, 0x00, 0x25, 0x01, 0x75, 0x01, 0x95, 0x08, 0x81, 0x02, 0xC0];
        assert!(!parse(&keyboard).is_usable());
    }
}
