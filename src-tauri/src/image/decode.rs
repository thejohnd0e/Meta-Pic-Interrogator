use std::io::Cursor;

use image::{DynamicImage, ImageFormat, ImageReader};

use crate::domain::{AppError, AppResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageLimits {
    pub max_width: u32,
    pub max_height: u32,
    pub max_pixels: u64,
}

impl Default for ImageLimits {
    fn default() -> Self {
        Self {
            max_width: 16_384,
            max_height: 16_384,
            max_pixels: 100_000_000,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Orientation {
    TopLeft,
    FlipHorizontal,
    Rotate180,
    FlipVertical,
    Transpose,
    Rotate90,
    Transverse,
    Rotate270,
}

#[derive(Debug)]
pub struct DecodedImage {
    pub image: DynamicImage,
    pub format: ImageFormat,
    pub width: u32,
    pub height: u32,
}

pub fn decode_supported(
    bytes: &[u8],
    limits: ImageLimits,
    orientation: Orientation,
) -> AppResult<DecodedImage> {
    let reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|error| AppError::LocalImage(error.to_string()))?;
    let format = reader
        .format()
        .ok_or_else(|| AppError::LocalImage("unknown image format".to_owned()))?;
    if !matches!(
        format,
        ImageFormat::Png
            | ImageFormat::Jpeg
            | ImageFormat::WebP
            | ImageFormat::Bmp
            | ImageFormat::Tiff
    ) {
        return Err(AppError::LocalImage("unsupported image format".to_owned()));
    }
    let (width, height) = reader
        .into_dimensions()
        .map_err(|error| AppError::LocalImage(error.to_string()))?;
    let pixels = u64::from(width)
        .checked_mul(u64::from(height))
        .ok_or_else(|| AppError::LocalImage("image dimensions overflow".to_owned()))?;
    if width > limits.max_width || height > limits.max_height || pixels > limits.max_pixels {
        return Err(AppError::LocalImage(
            "image exceeds configured limits".to_owned(),
        ));
    }
    let image = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|error| AppError::LocalImage(error.to_string()))?
        .decode()
        .map_err(|error| AppError::LocalImage(error.to_string()))?;
    let image = apply_orientation(image, orientation);
    let (width, height) = (image.width(), image.height());
    Ok(DecodedImage {
        image,
        format,
        width,
        height,
    })
}

fn apply_orientation(image: DynamicImage, orientation: Orientation) -> DynamicImage {
    match orientation {
        Orientation::TopLeft => image,
        Orientation::FlipHorizontal => image.fliph(),
        Orientation::Rotate180 => image.rotate180(),
        Orientation::FlipVertical => image.flipv(),
        Orientation::Transpose => image.rotate90().fliph(),
        Orientation::Rotate90 => image.rotate90(),
        Orientation::Transverse => image.rotate270().fliph(),
        Orientation::Rotate270 => image.rotate270(),
    }
}

#[cfg(test)]
mod tests {
    use image::{DynamicImage, ImageFormat, Rgba, RgbaImage};

    use super::{decode_supported, ImageLimits, Orientation};

    fn png_bytes(width: u32, height: u32) -> Vec<u8> {
        let image =
            DynamicImage::ImageRgba8(RgbaImage::from_pixel(width, height, Rgba([1, 2, 3, 255])));
        let mut bytes = Cursor::new(Vec::new());
        image
            .write_to(&mut bytes, ImageFormat::Png)
            .expect("fixture encodes");
        bytes.into_inner()
    }

    use std::io::Cursor;

    #[test]
    fn decodes_png_and_applies_limits() {
        let bytes = png_bytes(2, 3);
        let decoded = decode_supported(&bytes, ImageLimits::default(), Orientation::TopLeft)
            .expect("png decodes");
        assert_eq!((decoded.width, decoded.height), (2, 3));
    }

    #[test]
    fn rejects_oversized_images_before_decode() {
        let bytes = png_bytes(2, 3);
        let result = decode_supported(
            &bytes,
            ImageLimits {
                max_width: 1,
                ..ImageLimits::default()
            },
            Orientation::TopLeft,
        );
        assert!(result.is_err());
    }

    #[test]
    fn rejects_malformed_bytes() {
        assert!(
            decode_supported(b"not image", ImageLimits::default(), Orientation::TopLeft).is_err()
        );
    }

    #[test]
    fn decodes_every_supported_format_without_mutating_source() {
        let image =
            DynamicImage::ImageRgb8(image::RgbImage::from_pixel(2, 1, image::Rgb([10, 20, 30])));
        for format in [
            ImageFormat::Png,
            ImageFormat::Jpeg,
            ImageFormat::WebP,
            ImageFormat::Bmp,
            ImageFormat::Tiff,
        ] {
            let mut cursor = Cursor::new(Vec::new());
            image
                .write_to(&mut cursor, format)
                .expect("fixture encodes");
            let source = cursor.into_inner();
            let original = source.clone();
            let decoded = decode_supported(&source, ImageLimits::default(), Orientation::TopLeft)
                .expect("format decodes");
            assert_eq!((decoded.width, decoded.height), (2, 1));
            assert_eq!(source, original);
        }
    }

    #[test]
    fn applies_rotation_orientation() {
        let decoded = decode_supported(
            &png_bytes(2, 3),
            ImageLimits::default(),
            Orientation::Rotate90,
        )
        .expect("png decodes");
        assert_eq!((decoded.width, decoded.height), (3, 2));
    }
}
