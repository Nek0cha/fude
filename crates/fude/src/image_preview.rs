//! 画像と SVG のプレビュー（9d の設計書）。形式の判定、幅と高さ、下に添える文、本文の要素。

use std::sync::Arc;

use fude_core::text::OpenError;
use gpui_kit::*;

use crate::file_view::open_error_text;
use crate::theme::{Theme, ui_font};

/// エディタ列で開く画像の種類。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ImageKind {
    /// png / jpg / gif / webp。編集しない。
    Raster(ImageFormat),
    /// テキストとして編集もできる。
    Svg,
}

/// 拡張子から画像の種類を決める（大文字小文字を問わない）。画像でなければ None。
pub(crate) fn image_kind(rel: &str) -> Option<ImageKind> {
    let name = rel.rsplit('/').next().unwrap_or(rel);
    let (_, ext) = name.rsplit_once('.')?;
    Some(match ext.to_ascii_lowercase().as_str() {
        "png" => ImageKind::Raster(ImageFormat::Png),
        "jpg" | "jpeg" => ImageKind::Raster(ImageFormat::Jpeg),
        "gif" => ImageKind::Raster(ImageFormat::Gif),
        "webp" => ImageKind::Raster(ImageFormat::Webp),
        "svg" => ImageKind::Svg,
        _ => return None,
    })
}

/// ラスター画像の頭だけを読んで、幅と高さを求める。読めなければ None。
pub(crate) fn raster_size(bytes: &[u8]) -> Option<(u32, u32)> {
    image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format().ok()?.into_dimensions().ok()
}

/// 開くラスター画像の大きさの上限。
pub(crate) const MAX_RASTER_BYTES: usize = 50 * 1024 * 1024;

/// len バイトのラスター画像を開けないときの文。開けるなら None。
pub(crate) fn raster_error(len: usize) -> Option<String> {
    (len > MAX_RASTER_BYTES).then(|| open_error_text(OpenError::TooLarge(len as u64)))
}

/// 画像の下に添える文。size は幅と高さ（SVG や読めないときは None）、len はバイト数。
pub(crate) fn image_caption(size: Option<(u32, u32)>, len: usize) -> String {
    let bytes = match len {
        n if n < 1024 => format!("{n} B"),
        n if n < 1024 * 1024 => format!("{} KB", (n as f64 / 1024.0).round()),
        n => format!("{:.1} MB", n as f64 / (1024.0 * 1024.0)),
    };
    match size {
        Some((w, h)) => format!("{w} × {h} · {bytes}"),
        None => bytes,
    }
}

/// 本文の要素。画像を真ん中に、エディタ列に収まるよう縮めて出し（大きくはしない）、下に caption を添える。
pub(crate) fn image_body(image: Arc<Image>, caption: String, theme: &Theme) -> Div {
    let dim = theme.fg_dim;
    div()
        .size_full()
        .p(px(16.))
        .flex()
        .flex_col()
        .gap(px(8.))
        .font(ui_font())
        .child(
            div().flex_1().min_h_0().w_full().child(
                // GPUI は ID のある img だけ、gif や webp のコマを進める。
                img(image)
                    .id("fv-image")
                    .size_full()
                    .object_fit(ObjectFit::ScaleDown)
                    .with_fallback(move || {
                        div().size_full().flex().items_center().justify_center().text_color(dim).child("表示できない画像").into_any_element()
                    }),
            ),
        )
        .child(div().flex_shrink_0().w_full().flex().justify_center().text_size(px(12.)).text_color(dim).child(caption))
}

#[cfg(test)]
mod tests {
    use super::{ImageKind, MAX_RASTER_BYTES, image_caption, image_kind, raster_error, raster_size};
    use gpui_kit::ImageFormat;

    /// 3 × 2 の PNG。
    const PNG_3X2: [u8; 73] = [
        137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 3, 0, 0, 0, 2, 8, 2, 0, 0, 0, 18, 22, 241, 77,
        0, 0, 0, 16, 73, 68, 65, 84, 120, 156, 99, 248, 207, 192, 0, 65, 12, 112, 22, 0, 65, 210, 5, 251, 135, 240, 185, 72,
        0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
    ];

    #[test]
    fn kind_from_extension() {
        assert_eq!(image_kind("a/b.png"), Some(ImageKind::Raster(ImageFormat::Png)));
        assert_eq!(image_kind("b.JPG"), Some(ImageKind::Raster(ImageFormat::Jpeg)));
        assert_eq!(image_kind("b.jpeg"), Some(ImageKind::Raster(ImageFormat::Jpeg)));
        assert_eq!(image_kind("c.gif"), Some(ImageKind::Raster(ImageFormat::Gif)));
        assert_eq!(image_kind("d.webp"), Some(ImageKind::Raster(ImageFormat::Webp)));
        assert_eq!(image_kind("icons/e.Svg"), Some(ImageKind::Svg));
        assert_eq!(image_kind("f.bmp"), None);
        assert_eq!(image_kind("png"), None);
        assert_eq!(image_kind("dir.png/readme.md"), None);
    }

    #[test]
    fn size_from_header() {
        assert_eq!(raster_size(&PNG_3X2), Some((3, 2)));
        assert_eq!(raster_size(b"not an image"), None);
    }

    #[test]
    fn caption_text() {
        assert_eq!(image_caption(Some((1920, 1080)), 250_880), "1920 × 1080 · 245 KB");
        assert_eq!(image_caption(None, 512), "512 B");
        assert_eq!(image_caption(Some((3, 2)), 3 * 1024 * 1024 / 2), "3 × 2 · 1.5 MB");
    }

    #[test]
    fn raster_limit() {
        assert_eq!(raster_error(MAX_RASTER_BYTES), None);
        assert_eq!(raster_error(60 * 1024 * 1024).as_deref(), Some("大きすぎるため開けない（60.0 MB）"));
    }
}
