//! Icons: Lucide SVGs (ISC), some shared through fastframe-icons, and the
//! RusTone logo rendered for the window, taskbar and tray.

fastframe_icons::icons! {
    /// Every icon the interface draws.
    pub enum Icon {
        prefix: "rustone-icon-",
        directory: "../assets/icons/",
        Phone => "phone",
        PhoneOff => "phone-off",
        PhoneIncoming => "phone-incoming",
        PhoneOutgoing => "phone-outgoing",
        PhoneMissed => "phone-missed",
        Backspace => "delete",
        Dialpad => "grid-3x3",
        History => "history",
        Message => "message-circle",
        Video => "video",
        Contact => "contact",
        Users => lucide "users",
        Settings => lucide "settings",
        Back => lucide "arrow-left",
        Search => lucide "search",
        Sun => lucide "sun",
        Moon => lucide "moon",
        Monitor => lucide "monitor",
        Info => lucide "info",
        Lock => lucide "lock",
        ExternalLink => lucide "external-link",
    }
}

const LOGO_SVG: &[u8] = include_bytes!("../assets/rustone.svg");

/// The logo as straight RGBA, `size`×`size` pixels.
pub fn logo_rgba(size: usize) -> Vec<u8> {
    let opts = resvg::usvg::Options::default();
    let tree = resvg::usvg::Tree::from_data(LOGO_SVG, &opts).expect("logo SVG is valid");
    let side = u32::try_from(size.max(1)).unwrap_or(256);
    let mut pixmap = resvg::tiny_skia::Pixmap::new(side, side).expect("non-zero size");
    let scale = side as f32 / tree.size().width();
    resvg::render(&tree, resvg::tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
    // tiny-skia stores premultiplied alpha; the tray and window want straight.
    let mut rgba = pixmap.take();
    for px in rgba.as_chunks_mut::<4>().0 {
        let a = px[3];
        if a != 0 && a != 255 {
            for c in &mut px[..3] {
                *c = ((u16::from(*c) * 255 + u16::from(a) / 2) / u16::from(a)) as u8;
            }
        }
    }
    rgba
}

/// The window and taskbar icon.
pub fn window_icon() -> egui::IconData {
    const SIZE: usize = 64;
    egui::IconData { rgba: logo_rgba(SIZE), width: SIZE as u32, height: SIZE as u32 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logo_renders_at_tray_sizes() {
        for size in [16, 24, 32, 64] {
            let rgba = logo_rgba(size);
            assert_eq!(rgba.len(), size * size * 4);
            // The tile is opaque in the middle.
            let mid = (size / 2 * size + size / 2) * 4;
            assert_eq!(rgba[mid + 3], 255);
        }
    }
}
