use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{bail, Context, Result};
use hyper::{Client, StatusCode};
use hyper_tls::HttpsConnector;
use regex::Regex;
use tokio::{
    sync::mpsc::{self, UnboundedSender},
    time::sleep,
};

use crate::git_parsing::{parse_hash, parse_head, parse_log, parse_object, GitObject};

lazy_static::lazy_static! {
    static ref REGEX_OBJECT_PATH: Regex = Regex::new(r"[\da-f]{2}/[\da-f]{38}").unwrap();
}

const START_FILES: &[&str] = &[
    "info/exclude",
    "logs/HEAD",
    "objects/info/packs", // TODO: this does not seem to be present anymore?
    "config",
    "COMMIT_EDITMSG",
    "description",
    "FETCH_HEAD",
    "HEAD",
    "index",
    "ORIG_HEAD",
    "packed-refs",
    "refs/remotes/origin/HEAD", // guessing remote names seems pointless, it's `origin` 99% of the time
];

// TODO: brute-force files based on known and unknown branch names

#[derive(Debug)]
struct DownloadedFile {
    pub path: String,
    pub tx: UnboundedSender<DownloadedFile>,
}

pub async fn download_all(base_url: String, base_path: PathBuf, max_task_count: u16) {
    let mut cache = HashSet::<String>::new();

    // TODO: try out unbounded channel too
    // TODO: maybe just have a cli option that determines the limit of concurrent downloads instead?
    let (tx, mut rx) = mpsc::unbounded_channel();

    for &file in START_FILES {
        // let new_tx = tx.clone();
        // cache.download(file, new_tx);
        tx.send(DownloadedFile {
            path: file.into(),
            tx: tx.clone(),
        })
        .unwrap();
    }

    // drop the sender object so all senders can be out of scope by the end of the download
    drop(tx);

    // every time we downloaded a new file, see what other files we can derive from it
    let mut threads = vec![];
    while let Some(message) = rx.recv().await {
        // TODO: if this file is already downloaded, continue
        if cache.contains(&message.path) {
            // println!("Skipping download of file {file_name} as it's already downloaded");
            continue;
        }

        cache.insert(message.path.clone());

        let url = format!("{}{}", &base_url, &message.path);
        let base_path = base_path.clone();
        let handle = tokio::spawn(async move {
            let file_bytes = match download(&url).await {
                Ok(content) => content,
                Err(e) => {
                    println!("Error while downloading file {url}: {}", e);
                    return;
                }
            };

            println!("Downloaded '{}' ({} bytes)", message.path, file_bytes.len());

            // write this file to disk
            if let Err(e) = write_file(&base_path, &message.path, &file_bytes) {
                println!("Failed to write file {} to disk: {}", &message.path, e)
            }

            // match on the file name and queue new messages
            if let Err(e) = queue_new_references(message.path.as_str(), &file_bytes, message.tx) {
                println!("Error while trying to find new references: {e}");
            }
        });

        threads.push(handle);

        while threads.len() >= (max_task_count as usize) {
            // sleep
            sleep(Duration::from_millis(10)).await;

            // remove dead threads
            threads.retain(|h| !h.is_finished());
        }
    }
}

async fn download(url: &str) -> Result<Vec<u8>> {
    let client = Client::builder().build::<_, hyper::Body>(HttpsConnector::new());
    let resp = client.get(url.parse().unwrap()).await;
    match resp {
        Ok(resp) => match resp.status() {
            StatusCode::OK => {
                let bytes = hyper::body::to_bytes(resp).await.unwrap();
                Ok(bytes.to_vec())
            }
            StatusCode::NOT_FOUND => {
                bail!("Got 404 while trying to download {url}")
            }
            _ => {
                bail!(
                    "Error while trying to download {url}: status code is {}",
                    resp.status()
                )
            }
        },
        Err(e) => {
            bail!("Error while trying to download {url}: {e}");
        }
    }
}

fn write_file(base_path: &Path, message_name: &str, message_content: &[u8]) -> Result<()> {
    let path = base_path.join(".git").join(message_name);
    let path_parent = path
        .parent()
        .expect("There should be at least .git as parent");

    std::fs::create_dir_all(path_parent).with_context(|| {
        format!(
            "Error while trying to create directory {}",
            path_parent.to_string_lossy()
        )
    })?;
    std::fs::write(path, &message_content)
        .with_context(|| format!("Error while trying to write {} to disk", message_name))?;

    Ok(())
}

fn queue_new_references(
    name: &str,
    content: &[u8],
    tx: UnboundedSender<DownloadedFile>,
) -> Result<()> {
    match name {
        "HEAD" | "refs/remotes/origin/HEAD" => {
            let ref_path = parse_head(content)?;
            println!("\tFound ref path {ref_path}");

            tx.send(DownloadedFile {
                path: ref_path.into(),
                tx: tx.clone(),
            })
            .unwrap();
        }
        n if n.starts_with("refs/heads/") || n == "ORIG_HEAD" => {
            let hash = parse_hash(content)?;
            println!("\tFound object hash {hash}");

            tx.send(DownloadedFile {
                path: hash_to_url(hash),
                tx: tx.clone(),
            })
            .unwrap();
        }
        // TODO: handle FETCH_HEAD, detect branches
        // TODO: handle config, detect branches
        n if n.starts_with("logs/") => {
            let hashes = parse_log(content)?;

            println!("\tFound log with {} hashes", hashes.len());
            for hash in hashes {
                tx.send(DownloadedFile {
                    path: hash_to_url(&hash),
                    tx: tx.clone(),
                })
                .unwrap();
            }
        }
        n if n.starts_with("objects/") && REGEX_OBJECT_PATH.is_match(n) => {
            match parse_object(content)? {
                GitObject::Blob => {
                    println!("\tFound blob object");
                }
                GitObject::Tree(hashes) => {
                    println!("\tFound tree object with {} hashes", hashes.len());
                    for hash in hashes {
                        tx.send(DownloadedFile {
                            path: hash_to_url(&hash),
                            tx: tx.clone(),
                        })
                        .unwrap();
                    }
                }
                GitObject::Commit(hashes) => {
                    println!("\tFound commit object with {} hashes", hashes.len());
                    for hash in hashes {
                        tx.send(DownloadedFile {
                            path: hash_to_url(&hash),
                            tx: tx.clone(),
                        })
                        .unwrap();
                    }
                }
            }
        }
        n => {
            println!("\tNot using file '{n}' for anything right now");
        }
    }
    Ok(())
}

fn hash_to_url(hash: &str) -> String {
    assert_eq!(hash.len(), 40);
    let hash_start = &hash[0..2];
    let hash_end = &hash[2..];
    let path = format!("objects/{hash_start}/{hash_end}");
    path
}
