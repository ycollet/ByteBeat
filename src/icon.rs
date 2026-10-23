use eframe::egui::IconData;
use resvg::{tiny_skia, usvg};

pub fn load() -> Result<IconData, String> {
    let tree = usvg::Tree::from_data(
        include_bytes!("../assets/bytebeat.svg"),
        &usvg::Options::default(),
    )
    .map_err(|error| format!("Cannot parse application icon: {error}"))?;
    let size = tree.size().to_int_size();
    let mut pixmap = tiny_skia::Pixmap::new(size.width(), size.height())
        .ok_or_else(|| "Cannot allocate application icon".to_owned())?;
    resvg::render(
        &tree,
        tiny_skia::Transform::identity(),
        &mut pixmap.as_mut(),
    );
    let rgba = pixmap
        .pixels()
        .iter()
        .flat_map(|pixel| {
            // Native icons expect straight alpha, not tiny-skia's premultiplied alpha.
            let color = pixel.demultiply();
            [color.red(), color.green(), color.blue(), color.alpha()]
        })
        .collect();
    Ok(IconData {
        rgba,
        width: size.width(),
        height: size.height(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_svg_renders_native_icon() {
        let icon = load().expect("embedded application icon must render");
        assert_eq!((icon.width, icon.height), (256, 256));
        assert_eq!(icon.rgba.len(), 256 * 256 * 4);
        assert_eq!(&icon.rgba[..4], &[0, 0, 0, 0]);
        let pixel = |x: usize, y: usize| {
            let offset = (y * icon.width as usize + x) * 4;
            &icon.rgba[offset..offset + 4]
        };
        assert_eq!(pixel(128, 40), &[24, 27, 31, 255]);
        assert_eq!(pixel(56, 112), &[100, 224, 150, 255]);
        assert_eq!(pixel(104, 184), &[99, 184, 241, 255]);
    }
}
