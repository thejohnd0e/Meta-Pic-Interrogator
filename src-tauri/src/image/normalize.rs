use std::io::Cursor;

use image::ImageFormat;

use super::decode::DecodedImage;
use crate::domain::{AppError, AppResult};

#[derive(Debug, PartialEq, Eq)]
pub struct AiImageCopy {
    pub bytes: Vec<u8>,
    pub mime: &'static str,
    pub width: u32,
    pub height: u32,
}

pub fn normalize_for_ai(
    decoded: &DecodedImage,
    max_dimension: u32,
    max_bytes: usize,
) -> AppResult<AiImageCopy> {
    let image = if decoded.width > max_dimension || decoded.height > max_dimension {
        decoded.image.resize(
            max_dimension,
            max_dimension,
            image::imageops::FilterType::Lanczos3,
        )
    } else {
        decoded.image.clone()
    };
    let has_alpha = image.color().has_alpha();
    let (format, mime) = if has_alpha {
        (ImageFormat::Png, "image/png")
    } else {
        (ImageFormat::Jpeg, "image/jpeg")
    };
    let mut output = Cursor::new(Vec::new());
    image
        .write_to(&mut output, format)
        .map_err(|error| AppError::LocalImage(error.to_string()))?;
    let bytes = output.into_inner();
    if bytes.len() > max_bytes {
        return Err(AppError::LocalImage(
            "normalized image exceeds provider limit".to_owned(),
        ));
    }
    Ok(AiImageCopy {
        bytes,
        mime,
        width: image.width(),
        height: image.height(),
    })
}

#[cfg(test)]
mod tests {
    use image::{DynamicImage, ImageFormat, Rgba, RgbaImage};

    use super::normalize_for_ai;
    use crate::image::decode::{decode_supported, ImageLimits, Orientation};

    #[test]
    fn keeps_alpha_as_png_and_resizes() {
        let image = DynamicImage::ImageRgba8(RgbaImage::from_pixel(4, 2, Rgba([1, 2, 3, 100])));
        let mut bytes = std::io::Cursor::new(Vec::new());
        image
            .write_to(&mut bytes, ImageFormat::Png)
            .expect("fixture encodes");
        let source = bytes.into_inner();
        let decoded = decode_supported(&source, ImageLimits::default(), Orientation::TopLeft)
            .expect("fixture decodes");
        let normalized = normalize_for_ai(&decoded, 2, 10_000).expect("normalization succeeds");
        assert_eq!(normalized.mime, "image/png");
        assert_eq!((normalized.width, normalized.height), (2, 1));
    }
}
