use std::{path::Path, str};

use crate::domain::{AppError, AppResult, Provenance};

const SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedMetadata {
    pub description: String,
    pub provenance: Provenance,
    pub xmp: String,
}

fn chunk(kind: &[u8; 4], data: &[u8]) -> AppResult<Vec<u8>> {
    let length = u32::try_from(data.len())
        .map_err(|_| AppError::LocalMetadata("chunk too large".to_owned()))?;
    let mut bytes = Vec::with_capacity(data.len() + 12);
    bytes.extend_from_slice(&length.to_be_bytes());
    bytes.extend_from_slice(kind);
    bytes.extend_from_slice(data);
    let mut crc = crc32fast::Hasher::new();
    crc.update(kind);
    crc.update(data);
    bytes.extend_from_slice(&crc.finalize().to_be_bytes());
    Ok(bytes)
}

fn itxt(keyword: &str, text: &str) -> AppResult<Vec<u8>> {
    let mut data = Vec::new();
    data.extend_from_slice(keyword.as_bytes());
    data.extend_from_slice(b"\0\0\0\0\0");
    data.extend_from_slice(text.as_bytes());
    chunk(b"iTXt", &data)
}

fn escape_xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn xmp(description: &str, provenance: &Provenance) -> AppResult<String> {
    let json = serde_json::to_string(provenance)
        .map_err(|error| AppError::LocalMetadata(error.to_string()))?;
    Ok(format!("<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"><rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\"><rdf:Description xmlns:metapic=\"https://metapic.app/ns/interrogator/1.0/\"><metapic:description>{}</metapic:description><metapic:provenance>{}</metapic:provenance></rdf:Description></rdf:RDF></x:xmpmeta>", escape_xml(description), escape_xml(&json)))
}

pub fn write_metadata_png(
    input: &[u8],
    description: &str,
    provenance: &Provenance,
) -> AppResult<Vec<u8>> {
    if input.len() < 8 || &input[..8] != SIGNATURE {
        return Err(AppError::LocalMetadata("invalid PNG signature".to_owned()));
    }
    if description.trim().is_empty() {
        return Err(AppError::EmptyDescription);
    }
    let json = serde_json::to_string(provenance)
        .map_err(|error| AppError::LocalMetadata(error.to_string()))?;
    let xmp = xmp(description, provenance)?;
    let mut output = input[..8].to_vec();
    let mut offset = 8;
    let mut inserted = false;
    while offset + 12 <= input.len() {
        let length = usize::try_from(u32::from_be_bytes(
            input[offset..offset + 4]
                .try_into()
                .map_err(|_| AppError::LocalMetadata("invalid chunk length".to_owned()))?,
        ))
        .map_err(|_| AppError::LocalMetadata("invalid chunk length".to_owned()))?;
        if input.len() - offset < length + 12 || length > 16 * 1024 * 1024 {
            return Err(AppError::LocalMetadata("invalid PNG chunk".to_owned()));
        }
        let kind = &input[offset + 4..offset + 8];
        let data = &input[offset + 8..offset + 8 + length];
        let expected = u32::from_be_bytes(
            input[offset + 8 + length..offset + 12 + length]
                .try_into()
                .map_err(|_| AppError::LocalMetadata("invalid CRC".to_owned()))?,
        );
        let mut crc = crc32fast::Hasher::new();
        crc.update(kind);
        crc.update(data);
        if crc.finalize() != expected {
            return Err(AppError::LocalMetadata("PNG CRC mismatch".to_owned()));
        }
        if kind == b"IDAT" && !inserted {
            output.extend_from_slice(&itxt("Description", description)?);
            // Same text under the Stable Diffusion key so viewers that only
            // read `Parameters` (for example the Eagle PNG metadata plugin) show it.
            output.extend_from_slice(&itxt("Parameters", description)?);
            output.extend_from_slice(&itxt("MetaPic:Interrogator", &json)?);
            output.extend_from_slice(&itxt("XML:com.adobe.xmp", &xmp)?);
            inserted = true;
        }
        if kind != b"tEXt" && kind != b"zTXt" && kind != b"iTXt" && kind != b"eXIf" {
            output.extend_from_slice(&input[offset..offset + length + 12]);
        }
        offset += length + 12;
        if kind == b"IEND" {
            break;
        }
    }
    if !inserted {
        return Err(AppError::LocalMetadata("PNG has no IDAT".to_owned()));
    }
    Ok(output)
}

pub fn parse_metadata(bytes: &[u8]) -> AppResult<ParsedMetadata> {
    if bytes.len() < 8 || &bytes[..8] != SIGNATURE {
        return Err(AppError::LocalMetadata("invalid PNG signature".to_owned()));
    }
    let mut offset = 8;
    let mut description = None;
    let mut parameters = None;
    let mut provenance = None;
    let mut xmp_value = None;
    while offset + 12 <= bytes.len() {
        let length = usize::try_from(u32::from_be_bytes(
            bytes[offset..offset + 4]
                .try_into()
                .map_err(|_| AppError::LocalMetadata("invalid chunk".to_owned()))?,
        ))
        .map_err(|_| AppError::LocalMetadata("invalid chunk".to_owned()))?;
        if bytes.len() - offset < length + 12 {
            return Err(AppError::LocalMetadata("truncated PNG".to_owned()));
        }
        let kind = &bytes[offset + 4..offset + 8];
        let data = &bytes[offset + 8..offset + 8 + length];
        if kind == b"iTXt" {
            if let Some(first) = data.iter().position(|byte| *byte == 0) {
                let keyword = &data[..first];
                let mut separators = data[first + 1..]
                    .iter()
                    .enumerate()
                    .filter(|(_, byte)| **byte == 0)
                    .map(|(index, _)| first + 1 + index);
                let _compression_flag = separators.next();
                let _compression_method = separators.next();
                let _language = separators.next();
                if let Some(text_start) = separators.next() {
                    if let Ok(text) = str::from_utf8(&data[text_start + 1..]) {
                        match keyword {
                            b"Description" => description = Some(text.to_owned()),
                            b"Parameters" => parameters = Some(text.to_owned()),
                            b"MetaPic:Interrogator" => provenance = serde_json::from_str(text).ok(),
                            b"XML:com.adobe.xmp" => xmp_value = Some(text.to_owned()),
                            _ => {}
                        }
                    }
                }
            }
        }
        offset += length + 12;
    }
    match (description, parameters, provenance, xmp_value) {
        (Some(description), Some(parameters), Some(provenance), Some(xmp))
            if parameters == description =>
        {
            Ok(ParsedMetadata {
                description,
                provenance,
                xmp,
            })
        }
        _ => Err(AppError::LocalMetadata(
            "required metadata missing".to_owned(),
        )),
    }
}

pub fn save_verified_png(path: &Path, bytes: &[u8]) -> AppResult<()> {
    let temporary = path.with_extension("metapic.tmp.png");
    std::fs::write(&temporary, bytes)
        .map_err(|error| AppError::LocalMetadata(error.to_string()))?;
    if let Err(error) = parse_metadata(bytes) {
        let _ = std::fs::remove_file(&temporary);
        return Err(error);
    }
    std::fs::rename(temporary, path).map_err(|error| AppError::LocalMetadata(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{DynamicImage, Rgba, RgbaImage};

    fn provenance() -> Provenance {
        Provenance {
            schema_version: 1,
            provider: "test".to_owned(),
            model: "vision".to_owned(),
            preset_id: "p".to_owned(),
            preset_name: "Preset".to_owned(),
            preset_prompt: "prompt".to_owned(),
            created_at_utc: "2026-10-05T00:00:00Z".to_owned(),
            app_version: "0.1.0".to_owned(),
        }
    }

    #[test]
    fn writes_and_reparses_unicode_metadata() {
        let mut cursor = std::io::Cursor::new(Vec::new());
        DynamicImage::ImageRgba8(RgbaImage::from_pixel(1, 1, Rgba([1, 2, 3, 255])))
            .write_to(&mut cursor, image::ImageFormat::Png)
            .expect("fixture");
        let output = write_metadata_png(&cursor.into_inner(), "Привет <world> 🌍", &provenance())
            .expect("metadata");
        let parsed = parse_metadata(&output).expect("verify");
        assert_eq!(parsed.description, "Привет <world> 🌍");
        assert_eq!(parsed.provenance.provider, "test");
        assert!(parsed.xmp.contains("&lt;world&gt;"));
    }

    #[test]
    fn parameters_chunk_mirrors_description_for_eagle() {
        let mut cursor = std::io::Cursor::new(Vec::new());
        DynamicImage::ImageRgba8(RgbaImage::from_pixel(1, 1, Rgba([1, 2, 3, 255])))
            .write_to(&mut cursor, image::ImageFormat::Png)
            .expect("fixture");
        let output =
            write_metadata_png(&cursor.into_inner(), "Описание", &provenance()).expect("metadata");
        let needle = b"Parameters     ";
        let found = output
            .windows(needle.len())
            .position(|window| window == needle)
            .expect("Parameters iTXt present");
        let text = "Описание".as_bytes();
        assert_eq!(&output[found + needle.len()..][..text.len()], text);
    }

    #[test]
    fn rejects_corrupt_crc() {
        let mut bytes = SIGNATURE.to_vec();
        bytes.extend_from_slice(&0u32.to_be_bytes());
        bytes.extend_from_slice(b"IEND");
        bytes.extend_from_slice(&1u32.to_be_bytes());
        assert!(write_metadata_png(&bytes, "x", &provenance()).is_err());
    }
}
