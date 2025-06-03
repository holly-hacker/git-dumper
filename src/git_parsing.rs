use anyhow::{anyhow, bail, Result};
use miniz_oxide::inflate::TINFLStatus;
use regex::Regex;
use std::{collections::HashSet, fmt::Write};

lazy_static::lazy_static! {
    static ref REGEX_HASH: Regex = Regex::new(r"^[a-f\d]{40}$").unwrap();
    static ref REGEX_REFS_PATH: Regex = Regex::new(r"^refs/heads/(\S+)$").unwrap();
}

const EMPTY_HASH: &str = "0000000000000000000000000000000000000000";

pub enum GitObject {
    Tree(Vec<String>),
    Commit(Vec<String>),
    Blob,
}

pub fn parse_head(data: &[u8]) -> Result<&str> {
    let content = std::str::from_utf8(data)?;

    if !content.starts_with("ref: ") {
        bail!("HEAD file must start with \"ref: \"");
    }

    let content = content[5..].trim_end();

    if !REGEX_REFS_PATH.is_match(content) {
        bail!("Failed to match refs path in HEAD file");
    }

    // check for potential path traversal
    // a normal git setup should never emit paths with `..` segments
    if content.split(['/', '\\']).any(|segment| segment == "..") {
        bail!(
            "Unexpected path traversal detected in HEAD file: {}",
            content
        );
    }

    Ok(content)
}

pub fn parse_hash(data: &[u8]) -> Result<&str> {
    let content = std::str::from_utf8(data)?;
    let content = content.trim_end();

    if !REGEX_HASH.is_match(content) {
        bail!("Failed to match hash");
    }

    Ok(content)
}

pub fn parse_object(data: &[u8]) -> Result<GitObject> {
    let peek = peek_object_type(data)?;
    match peek {
        [b'b', b'l', b'o', b'b', _, _] => Ok(GitObject::Blob),
        [b't', b'r', b'e', b'e', _, _] => {
            let decompressed = miniz_oxide::inflate::decompress_to_vec_zlib(data)
                .map_err(|e| anyhow!("Problem while decompressing git object: {}", e))?;
            let decompressed = decompressed.as_slice();

            let mut hashes = vec![];

            // TODO: this is ugly, use a slice-based approach instead
            let mut decompressed_iter = split_object_at_zero(decompressed)?.iter().peekable();
            while decompressed_iter.peek().is_some() {
                let bytes: Vec<u8> = (&mut decompressed_iter)
                    .skip_while(|&&b| b != b'\0')
                    .skip(1)
                    .take(0x14)
                    .cloned()
                    .collect();
                hashes.push(slice_to_hex(&bytes));
            }

            Ok(GitObject::Tree(hashes))
        }
        [b'c', b'o', b'm', b'm', b'i', b't'] => {
            let decompressed = miniz_oxide::inflate::decompress_to_vec_zlib(data)
                .map_err(|e| anyhow!("Problem while decompressing git object: {}", e))?;

            let decompressed = split_object_at_zero(&decompressed)?;
            let commit_message = String::from_utf8_lossy(decompressed);

            let hashes = commit_message
                .lines()
                .take_while(|&line| !line.trim().is_empty())
                .filter_map(|line| match line.split_once(' ') {
                    Some(("tree", hash)) => Some(hash.into()),
                    Some(("parent", hash)) => Some(hash.into()),
                    _ => None,
                })
                .collect();

            Ok(GitObject::Commit(hashes))
        }
        _ => bail!(
            "Unknown git object header: {}",
            String::from_utf8_lossy(&peek)
        ),
    }
}

pub fn parse_log(data: &[u8]) -> Result<HashSet<String>> {
    let mut set = HashSet::new();

    let content = String::from_utf8_lossy(data);

    for line in content.lines() {
        let (hash1, rest) = line.split_once(' ').unwrap_or(("", ""));
        let (hash2, _) = rest.split_once(' ').unwrap_or(("", ""));

        if REGEX_HASH.is_match(hash1) && hash1 != EMPTY_HASH {
            set.insert(hash1.into());
        }

        if REGEX_HASH.is_match(hash2) && hash2 != EMPTY_HASH {
            set.insert(hash2.into());
        }
    }

    Ok(set)
}

// pub fn

fn peek_object_type(data: &[u8]) -> Result<[u8; 6]> {
    let mut array = [0u8; 6];
    match miniz_oxide::inflate::decompress_slice_iter_to_slice(
        &mut array,
        data.chunks(16),
        true,
        true,
    ) {
        Ok(_) | Err(TINFLStatus::HasMoreOutput) => Ok(array),
        Err(e) => bail!("Error while decompressing object file for peek: {:?}", e),
    }
}

fn split_object_at_zero(data: &[u8]) -> Result<&[u8]> {
    let idx_zero = data
        .iter()
        .enumerate()
        .find(|(_, &val)| val == b'\0')
        .map(|(idx, _)| idx)
        .ok_or_else(|| anyhow!("Malformed object file, could not find null separator"))?;
    let data = &data[idx_zero + 1..];
    Ok(data)
}

fn slice_to_hex(data: &[u8]) -> String {
    let mut s = String::with_capacity(data.len() * 2);
    for byte in data {
        write!(s, "{:02x}", byte).expect("writing hex should not fail");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_commit_blob() {
        let bytes = include_bytes!("../test-data/object-blob");
        let parsed = parse_object(bytes).unwrap();
        assert!(matches!(parsed, GitObject::Blob));
    }

    #[test]
    fn parse_tree_object() {
        let bytes = include_bytes!("../test-data/object-tree");
        let parsed = parse_object(bytes).unwrap();
        assert!(matches!(parsed, GitObject::Tree(_)));

        if let GitObject::Tree(vec) = parsed {
            assert_eq!(
                vec,
                vec![
                    "93748a31e8df89b80ab5ebe4ad19ea62899a28fa".to_string(),
                    "920512d27e4df0c79ca4a929bc5d4254b3d05c4c".to_string(),
                    "f5463e0d810357c84bdb956dcfe70b8015d6fb24".to_string(),
                ]
            );
        }
    }

    #[test]
    fn parse_commit_object() {
        let bytes = include_bytes!("../test-data/object-commit");
        let parsed = parse_object(bytes).unwrap();
        assert!(matches!(parsed, GitObject::Commit(_)));

        if let GitObject::Commit(vec) = parsed {
            assert_eq!(
                vec,
                vec![
                    "faf660b3b793f359495ad23ea2c449da6b3b64a0".to_string(),
                    "1712bc7d3a0e6cf9920541e616310bd30f431728".to_string(),
                ]
            );
        }
    }
}
