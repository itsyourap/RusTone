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
        Eye => lucide "eye",
        EyeOff => lucide "eye-off",
        Plus => lucide "plus",
        Pencil => lucide "pencil",
        Trash => lucide "trash-2",
        Check => lucide "check",
        Smartphone => lucide "smartphone",
        Mic => lucide "mic",
        MicOff => "mic-off",
        Speaker => lucide "volume-2",
        SpeakerOff => lucide "volume-x",
        Bell => "bell",
        Star => "star",
        Send => "send-horizontal",
        VideoOff => "video-off",
        Record => "circle-dot",
        Maximize => lucide "maximize-2",
        Minimize => lucide "minimize-2",
        More => lucide "ellipsis",
        Copy => lucide "copy",
        BellOff => "bell-off",
        Pause => lucide "pause",
        Play => lucide "play",
        Transfer => "phone-forwarded",
        AddCall => "user-plus",
        Close => lucide "x",
        Up => lucide "chevron-up",
        Down => lucide "chevron-down",
        Dashboard => "layout-dashboard",
        Logs => "scroll-text",
        Network => "arrow-left-right",
        Activity => "activity",
        Waveform => "audio-waveform",
        Bug => "bug",
        Outgoing => "arrow-up-right",
        Incoming => "arrow-down-left",
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

/// The logo greyed out and faded, for the tray while not registered.
pub fn logo_rgba_offline(size: usize) -> Vec<u8> {
    let mut rgba = logo_rgba(size);
    for px in rgba.as_chunks_mut::<4>().0 {
        let grey = ((u16::from(px[0]) * 3 + u16::from(px[1]) * 6 + u16::from(px[2])) / 10) as u8;
        px[0] = grey;
        px[1] = grey;
        px[2] = grey;
        px[3] = (u16::from(px[3]) * 3 / 4) as u8;
    }
    rgba
}

/// The logo with a red dot in its top-right corner, for the tray after
/// missed calls.
pub fn logo_rgba_missed(size: usize) -> Vec<u8> {
    let mut rgba = logo_rgba(size);
    let radius = size as f32 * 0.22;
    let (cx, cy) = (size as f32 - radius, radius);
    for y in 0..size {
        for x in 0..size {
            let d = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
            // White ring, then the red dot, edges softened by a pixel.
            let (color, edge): ([u8; 3], f32) = if d <= radius - 1.0 { ([0xe5, 0x39, 0x35], radius - 1.0) } else { ([0xff, 0xff, 0xff], radius + 0.5) };
            let coverage = (edge + 0.5 - d).clamp(0.0, 1.0);
            if coverage > 0.0 {
                let px = &mut rgba[(y * size + x) * 4..][..4];
                for (c, new) in px[..3].iter_mut().zip(color) {
                    *c = (f32::from(*c) * (1.0 - coverage) + f32::from(new) * coverage) as u8;
                }
                px[3] = px[3].max((coverage * 255.0) as u8);
            }
        }
    }
    rgba
}

/// The logo as a PNG, for notifications.
pub fn logo_png(size: usize) -> Vec<u8> {
    let side = u32::try_from(size.max(1)).unwrap_or(256);
    let mut pixmap = resvg::tiny_skia::Pixmap::new(side, side).expect("non-zero size");
    let tree = resvg::usvg::Tree::from_data(LOGO_SVG, &resvg::usvg::Options::default()).expect("logo SVG is valid");
    let scale = side as f32 / tree.size().width();
    resvg::render(&tree, resvg::tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
    pixmap.encode_png().unwrap_or_default()
}

/// The logo tile's blue, for the notification icon's background.
pub const LOGO_BLUE: [u8; 3] = [0x3b, 0x5b, 0xdb];

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
        let missed = logo_rgba_missed(32);
        let dot = (6 * 32 + 25) * 4;
        assert!(missed[dot] > 0xc0 && missed[dot + 1] < 0x80, "red dot top right");
        assert!(logo_png(64).starts_with(&[0x89, b'P', b'N', b'G']), "a PNG");
    }
}
