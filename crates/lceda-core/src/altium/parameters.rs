//! AD parameter blocks may mix legacy MBCS values and `%UTF8%` overrides.
//! Decode each field separately; decoding the entire block as GBK corrupts UTF-8.
use crate::{Error, Result};
use encoding_rs::{GBK, WINDOWS_1252};

pub(crate) type Parameters = Vec<(String, String)>;

pub(crate) fn decode(bytes: &[u8]) -> Result<Parameters> {
    let bytes = bytes.strip_suffix(&[0]).unwrap_or(bytes);
    if bytes.contains(&0) { return Err(Error::msg("AD 参数含意外 NUL")); }
    let western = bytes.split(|b| *b == b'|').any(|p| p.eq_ignore_ascii_case(b"USEMBCS=F"));
    bytes.split(|b| *b == b'|').filter(|p| !p.is_empty()).map(|part| {
        let equal = part.iter().position(|b| *b == b'=').ok_or_else(|| Error::msg("AD 参数缺少等号"))?;
        let key = std::str::from_utf8(&part[..equal]).map_err(|_| Error::msg("AD 参数名称无效"))?;
        let value = &part[equal + 1..];
        let text = if key.to_ascii_uppercase().starts_with("%UTF8%") {
            std::str::from_utf8(value).map_err(|_| Error::msg("AD UTF-8 参数无效"))?.to_string()
        } else if western { WINDOWS_1252.decode(value).0.into_owned() }
        else {
            let (text, _, errors) = GBK.decode(value);
            if errors { WINDOWS_1252.decode(value).0.into_owned() } else { text.into_owned() }
        };
        Ok((key.to_string(), text))
    }).collect()
}

pub(crate) fn value<'a>(parameters: &'a Parameters, name: &str) -> Option<&'a str> {
    let unicode = format!("%UTF8%{name}");
    parameters.iter().find(|(key, _)| key.eq_ignore_ascii_case(&unicode))
        .or_else(|| parameters.iter().find(|(key, _)| key.eq_ignore_ascii_case(name)))
        .map(|(_, value)| value.as_str())
}

/// Patch just changed fields, preserving original bytes, encodings and separators.
pub(crate) fn patch(bytes: &[u8], updated: &Parameters) -> Result<Vec<u8>> {
    let old = decode(bytes)?;
    let western = old.iter().any(|(k,v)| k.eq_ignore_ascii_case("USEMBCS") && v == "F");
    let encode = |key: &str, text: &str| -> Result<Vec<u8>> {
        if key.to_ascii_uppercase().starts_with("%UTF8%") { return Ok(text.as_bytes().to_vec()); }
        let (bytes, _, errors) = if western { WINDOWS_1252.encode(text) } else { GBK.encode(text) };
        if errors { return Err(Error::msg(format!("AD 参数 {key} 无法使用原库编码写入"))); }
        Ok(bytes.into_owned())
    };
    let mut output = Vec::new();
    for (i, part) in bytes.strip_suffix(&[0]).unwrap_or(bytes).split(|b| *b == b'|').enumerate() {
        if let Some(equal) = part.iter().position(|b| *b == b'=') {
            let key = std::str::from_utf8(&part[..equal]).map_err(|_| Error::msg("AD 参数名称无效"))?;
            if !updated.iter().any(|(k,_)| k.eq_ignore_ascii_case(key)) { continue; }
        }
        if i > 0 { output.push(b'|'); }
        let changed = part.iter().position(|b| *b == b'=').and_then(|equal| {
            let key = std::str::from_utf8(&part[..equal]).ok()?;
            let new = updated.iter().find(|(k,_)| k.eq_ignore_ascii_case(key))?;
            let previous = old.iter().find(|(k,_)| k.eq_ignore_ascii_case(key))?;
            (new.1 != previous.1).then_some((equal, key, new.1.as_str()))
        });
        if let Some((equal, key, text)) = changed {
            output.extend_from_slice(&part[..=equal]); output.extend(encode(key, text)?);
        } else { output.extend_from_slice(part); }
    }
    for (key, text) in updated {
        if old.iter().any(|(k,_)| k.eq_ignore_ascii_case(key)) { continue; }
        output.push(b'|'); output.extend_from_slice(key.as_bytes()); output.push(b'='); output.extend(encode(key, text)?);
    }
    output.push(0);
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mixed_encoding_overrides_and_patches_preserve_unknown_bytes() {
        let mut raw = b"|UseMBCS=T|%UTF8%FontName1=".to_vec();
        raw.extend_from_slice("楷体".as_bytes()); raw.extend_from_slice(b"|||FontName1=\xbf\xac\xcc\xe5|CompCount=1|Opaque=\x80\0");
        let mut fields = decode(&raw).unwrap();
        assert_eq!(value(&fields, "FONTNAME1"), Some("楷体"));
        assert_eq!(patch(&raw, &fields).unwrap(), raw);
        fields.iter_mut().find(|(k,_)| k == "CompCount").unwrap().1 = "2".into();
        let changed = patch(&raw, &fields).unwrap();
        assert_eq!(changed, raw.windows(11).position(|p| p == b"CompCount=1").map(|i| { let mut b=raw.clone(); b[i+10]=b'2'; b }).unwrap());
        assert!(decode(b"|%UTF8%Text=\xff\0").is_err());
    }
}
