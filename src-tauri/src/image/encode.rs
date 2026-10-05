use std::io::Cursor;

use image::{DynamicImage, ImageFormat};

use crate::domain::{AppError, AppResult};

pub fn encode_clean_png(image: &DynamicImage) -> AppResult<Vec<u8>> {
    let mut output = Cursor::new(Vec::new());
    image
        .to_rgba8()
        .write_to(&mut output, ImageFormat::Png)
        .map_err(|error| AppError::LocalImage(error.to_string()))?;
    Ok(output.into_inner())
}

#[cfg(test)]
mod tests {
    use image::{DynamicImage, Rgba, RgbaImage};

    use super::encode_clean_png;

    #[test]
    fn output_is_png_without_source_bytes() {
        let image = DynamicImage::ImageRgba8(RgbaImage::from_pixel(1, 1, Rgba([255, 0, 0, 255])));
        let output = encode_clean_png(&image).expect("png encodes");
        assert_eq!(&output[..8], b"\x89PNG\r\n\x1a\n");
    }
}
