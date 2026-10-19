//! Image previews: PNG, JPEG, GIF, WebP, BMP, TIFF and SVG decoded off the main thread.
use anyhow::{Context as _, Result, bail};
use give::{
    git::{ChangedFile, Repository},
    review::Review,
};
use gpui::{ImageFormat, RenderImage};
use image::Frame;
use resvg::{tiny_skia, usvg};
use std::{
    path::Path,
    sync::{Arc, LazyLock},
};

const IMAGE_LIMIT: u64 = 64 * 1024 * 1024;
/// SVGs rasterize at twice their size so they stay sharp on Retina displays.
pub const SVG_SCALE: f32 = 2.;
/// The longest rasterized SVG side; larger drawings render at a smaller scale.
const SVG_MAX_SIDE: f32 = 8192.;

pub fn format(path: &Path) -> Option<ImageFormat> {
    let ext = path.extension()?.to_string_lossy().to_lowercase();
    Some(match ext.as_str() {
        "png" => ImageFormat::Png,
        "jpg" | "jpeg" => ImageFormat::Jpeg,
        "gif" => ImageFormat::Gif,
        "webp" => ImageFormat::Webp,
        "bmp" => ImageFormat::Bmp,
        "tif" | "tiff" => ImageFormat::Tiff,
        "svg" => ImageFormat::Svg,
        _ => return None,
    })
}

/// A decoded image and the size it is drawn at, in logical pixels.
pub struct Decoded {
    pub image: Arc<RenderImage>,
    pub width: f32,
    pub height: f32,
}

/// Reads the shown version of `file`: the working tree, or the branch head in a review.
pub fn load(repo: &Repository, file: &ChangedFile, review: Option<&Review>) -> Result<Decoded> {
    let format = format(&file.path).context("not an image")?;
    let bytes = match review {
        Some(review) => {
            if file.index == 'D' {
                bail!("Image deleted in this branch.");
            }
            review.head_blob(repo, &file.path)?
        }
        None => {
            if file.index == 'D' || file.worktree == 'D' {
                bail!("Image deleted — nothing to show.");
            }
            let path = repo.root.join(&file.path);
            if std::fs::metadata(&path)?.len() > IMAGE_LIMIT {
                bail!("Image is larger than 64 MiB.");
            }
            std::fs::read(path)?
        }
    };
    decode(bytes, format)
}

fn decode(bytes: Vec<u8>, format: ImageFormat) -> Result<Decoded> {
    let raster = match format {
        ImageFormat::Svg => None,
        ImageFormat::Png => Some(image::ImageFormat::Png),
        ImageFormat::Jpeg => Some(image::ImageFormat::Jpeg),
        ImageFormat::Gif => Some(image::ImageFormat::Gif),
        ImageFormat::Webp => Some(image::ImageFormat::WebP),
        ImageFormat::Bmp => Some(image::ImageFormat::Bmp),
        ImageFormat::Tiff => Some(image::ImageFormat::Tiff),
    };
    if let Some(raster) = raster {
        // Animated GIFs show their first frame.
        let mut buffer = image::load_from_memory_with_format(&bytes, raster)?.into_rgba8();
        for pixel in buffer.chunks_exact_mut(4) {
            pixel.swap(0, 2);
        }
        let (width, height) = buffer.dimensions();
        return Ok(Decoded {
            image: Arc::new(RenderImage::new(vec![Frame::new(buffer)])),
            width: width as f32,
            height: height as f32,
        });
    }
    static OPTIONS: LazyLock<usvg::Options<'static>> = LazyLock::new(|| {
        let mut options = usvg::Options::default();
        Arc::make_mut(&mut options.fontdb).load_system_fonts();
        options
    });
    let tree = usvg::Tree::from_data(&bytes, &OPTIONS)?;
    let size = tree.size();
    let scale = SVG_SCALE.min(SVG_MAX_SIDE / size.width().max(size.height()));
    let width = (size.width() * scale).ceil().max(1.) as u32;
    let height = (size.height() * scale).ceil().max(1.) as u32;
    let mut pixmap = tiny_skia::Pixmap::new(width, height).context("SVG is too large")?;
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    let mut buffer =
        image::RgbaImage::from_raw(width, height, pixmap.take()).context("SVG pixel buffer")?;
    // tiny-skia is premultiplied RGBA; GPUI draws straight BGRA.
    for pixel in buffer.chunks_exact_mut(4) {
        pixel.swap(0, 2);
        if pixel[3] > 0 {
            let alpha = pixel[3] as f32 / 255.;
            for channel in &mut pixel[..3] {
                *channel = (*channel as f32 / alpha).round().min(255.) as u8;
            }
        }
    }
    Ok(Decoded {
        image: Arc::new(RenderImage::new(vec![Frame::new(buffer)])),
        width: size.width(),
        height: size.height(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn svg_renders_sharp_in_bgra_at_its_own_size() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="4"><rect width="10" height="4" fill="#ff0000"/></svg>"##;
        let decoded = decode(svg.to_vec(), ImageFormat::Svg).unwrap();
        assert_eq!((decoded.width, decoded.height), (10., 4.));
        let size = decoded.image.size(0);
        assert_eq!((size.width.0, size.height.0), (20, 8));
        assert_eq!(&decoded.image.as_bytes(0).unwrap()[..4], &[0, 0, 255, 255]);
    }

    #[test]
    fn png_decodes_to_bgra() {
        let mut png = Vec::new();
        image::RgbaImage::from_pixel(3, 2, image::Rgba([255, 0, 0, 255]))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let decoded = decode(png, ImageFormat::Png).unwrap();
        assert_eq!((decoded.width, decoded.height), (3., 2.));
        assert_eq!(&decoded.image.as_bytes(0).unwrap()[..4], &[0, 0, 255, 255]);
    }
}
