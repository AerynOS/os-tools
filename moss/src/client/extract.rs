use std::{
    fs::File,
    io::{self, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

use fs_err as fs;
use stone::{StoneDecodedPayload, StoneReadError};
use thiserror::Error;
use tui::{ProgressBar, ProgressStyle};

use crate::{
    Installation,
    client::{self, blit_from_package, cache::asset_path, create_root_links},
    installation,
    package::{self, MissingMetaFieldError},
    state::Selection,
};

pub fn extract(stones: Vec<PathBuf>) -> Result<(), Error> {
    // Begin unpack
    fs::create_dir_all(".stoneStore")?;

    let content_store = PathBuf::from(".stoneStore");
    let install_path = Path::new(".");
    let installation = Installation::open(install_path, None)?;

    for path in stones {
        let rdr = File::open(&path).map_err(Error::IO)?;
        let mut reader = stone::read(rdr).map_err(Error::Format)?;

        let payloads = reader.payloads()?.collect::<Result<Vec<_>, _>>()?;
        let content = payloads.iter().find_map(StoneDecodedPayload::content);
        let layouts = payloads.iter().find_map(StoneDecodedPayload::layout);
        let meta = payloads
            .iter()
            .find_map(StoneDecodedPayload::meta)
            .ok_or(Error::MissingMeta)?;

        let pkg = package::Meta::from_stone_payload(&meta.body).map_err(Error::MalformedMeta)?;
        let extraction_root = PathBuf::from(pkg.id().to_string());

        println!("Extract: {path:?} -> {extraction_root:?}");

        let selection = Selection {
            package: pkg.id().into(),
            explicit: true,
            reason: None,
        };

        // Cleanup old extraction root
        if extraction_root.exists() {
            fs::remove_dir_all(&extraction_root)?;
        }

        fs::create_dir_all(&extraction_root)?;

        create_root_links(&extraction_root)?;
        create_root_links(&installation.isolation_dir())?;

        fs::create_dir_all(installation.assets_path("v2"))?;

        let content_dir = installation.cache_path("content");
        let content_path = content_dir.join(pkg.id().to_string());

        fs::create_dir_all(&content_dir)?;

        if let Some(content) = content {
            let mut content_file = File::options()
                .read(true)
                .write(true)
                .create(true)
                .truncate(true)
                .open(&content_path)?;

            let _progress = ProgressBar::new(content.header.plain_size).with_style(
                ProgressStyle::with_template("|{bar:20.cyan/bue}| {percent}%")
                    .unwrap()
                    .progress_chars("■≡=- "),
            );
            reader.unpack_content(content, &mut content_file)?;

            // Extract all indices from the `.stoneContent` into hash-indexed unique files
            payloads
                .iter()
                .filter_map(StoneDecodedPayload::index)
                .flat_map(|p| &p.body)
                .map(|idx| {
                    let path = asset_path(&installation, &format!("{:02x}", idx.digest));

                    // This asset already exists
                    if path.exists() {
                        return Ok(());
                    }
                    // Create parent dir
                    if let Some(parent) = path.parent() {
                        fs::create_dir_all(parent)?;
                    }

                    // Split file reader over index range
                    let mut file = &content_file;
                    file.seek(SeekFrom::Start(idx.start))?;
                    let mut split_file = (&mut file).take(idx.end - idx.start);

                    let mut output = File::create(&path)?;

                    io::copy(&mut split_file, &mut output)?;

                    Ok(())
                })
                .collect::<Result<Vec<_>, Error>>()?;

            fs::remove_file(&content_path)?;
        }

        if let Some(layouts) = layouts {
            let layout_record = layouts.body.clone().into_iter().collect::<Vec<_>>();
            blit_from_package(
                &selection.package,
                layout_record,
                &installation,
                &extraction_root.canonicalize()?,
            )?;
        }
    }

    // Clean up.
    fs::remove_dir_all(content_store)?;
    fs::remove_dir_all(Path::new(".").join(".moss"))?;

    Ok(())
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("client")]
    Client(#[from] client::Error),

    #[error("Missing metadata")]
    MissingMeta,

    #[error("malformed meta")]
    MalformedMeta(#[from] MissingMetaFieldError),

    #[error("io")]
    IO(#[from] io::Error),

    #[error("stone format")]
    Format(#[from] StoneReadError),

    #[error("installation")]
    Installation(#[from] installation::Error),
}
