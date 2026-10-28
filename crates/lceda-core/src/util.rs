use std::path::Path;

pub fn sanitize_filename(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    let trimmed = cleaned.trim_matches([' ', '.'].as_slice());
    if trimmed.is_empty() {
        "component".into()
    } else {
        trimmed.to_string()
    }
}

/// `STM32F030C8T6_C23922_ST(意法半导体)`
pub fn part_stem(mpn: &str, lcsc: Option<&str>, manufacturer: &str) -> String {
    let mut parts = Vec::new();
    let name = sanitize_filename(mpn);
    if !name.is_empty() && name != "component" {
        parts.push(name);
    }
    if let Some(id) = lcsc.map(str::trim).filter(|s| !s.is_empty()) {
        parts.push(id.to_string());
    }
    let mfr = sanitize_filename(manufacturer);
    if !mfr.is_empty() && mfr != "component" {
        parts.push(mfr);
    }
    if parts.is_empty() {
        "component".into()
    } else {
        parts.join("_")
    }
}

/// Unique name; suffixes `_2`, `_3` on collision.
pub fn unique_name(name: &str, used: &mut std::collections::HashSet<String>) -> String {
    let base = name.trim();
    let base = if base.is_empty() { "component" } else { base };
    if used.insert(base.to_string()) {
        return base.to_string();
    }
    let mut index = 2usize;
    loop {
        let candidate = format!("{base}_{index}");
        if used.insert(candidate.clone()) {
            return candidate;
        }
        index += 1;
    }
}

/// Unique 31-char CFB storage name; suffixes `_2`, `_3` on collision.
pub fn unique_altium_section_key(name: &str, used: &mut std::collections::HashSet<String>) -> String {
    let base = altium_section_key(name);
    if used.insert(base.clone()) {
        return base;
    }
    let mut index = 2usize;
    loop {
        let suffix = format!("_{index}");
        let max_len = 31usize.saturating_sub(suffix.len());
        let prefix: String = base.chars().take(max_len.max(1)).collect();
        let candidate = format!("{prefix}{suffix}");
        if used.insert(candidate.clone()) {
            return candidate;
        }
        index += 1;
    }
}

/// Altium compound storage names are 31 chars, ASCII-ish.
pub fn altium_section_key(name: &str) -> String {
    let ascii: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.' {
                c
            } else if c == '/' {
                '_'
            } else {
                '_'
            }
        })
        .collect();
    let trimmed = ascii.trim_matches('_');
    let base = if trimmed.is_empty() { "component" } else { trimmed };
    base.chars().take(31).collect()
}

pub fn ensure_parent(path: &Path) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    Ok(())
}

pub fn looks_like_step(bytes: &[u8]) -> bool {
    let head = std::str::from_utf8(bytes.get(..64).unwrap_or(bytes)).unwrap_or("");
    head.contains("ISO-10303") || head.contains("STEP")
}

pub fn unique_id() -> String {
    use std::sync::{atomic::{AtomicU64, Ordering}, OnceLock};
    // Native AD IDs are eight uppercase letters. A clock-only seed can repeat
    // across Windows threads, sharing temporary directories and CAD IDs.
    // Permute a process-wide counter over all 26^8 IDs instead (not a secret).
    const SPACE: u64 = 26_u64.pow(8);
    static SEED: OnceLock<u64> = OnceLock::new();
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let seed = *SEED.get_or_init(|| {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64).unwrap_or(1);
        (nanos ^ u64::from(std::process::id()).rotate_left(32)) % SPACE
    });
    // 104729 is coprime to 26: distinct indices have distinct eight-letter IDs
    // until the complete 208-billion-ID space is exhausted in one process.
    let mut value = (seed + (NEXT.fetch_add(1, Ordering::Relaxed) % SPACE) * 104729) % SPACE;
    let mut out = [b'A'; 8];
    for c in out.iter_mut().rev() {
        *c += (value % 26) as u8;
        value /= 26;
    }
    String::from_utf8(out.to_vec()).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_ids_remain_distinct_under_concurrent_generation() {
        let ids = std::thread::scope(|scope| {
            let workers: Vec<_> = (0..8).map(|_| scope.spawn(|| (0..10000).map(|_| unique_id()).collect::<Vec<_>>())).collect();
            workers.into_iter().flat_map(|worker| worker.join().unwrap()).collect::<Vec<_>>()
        });
        assert!(ids.iter().all(|id| id.len() == 8 && id.bytes().all(|b| b.is_ascii_uppercase())));
        assert_eq!(ids.iter().collect::<std::collections::HashSet<_>>().len(), 80000);
    }

    #[test]
    fn sanitize_strips_bad_chars() {
        assert_eq!(sanitize_filename(r#"A<>:"/\|?*B"#), "A_________B");
        assert_eq!(sanitize_filename("   "), "component");
        assert_eq!(
            part_stem("STM32F030C8T6", Some("C23922"), "ST(意法半导体)"),
            "STM32F030C8T6_C23922_ST(意法半导体)"
        );
    }
}
