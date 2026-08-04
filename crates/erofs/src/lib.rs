// SPDX-FileCopyrightText: 2026 AerynOS Developers
// SPDX-License-Identifier: MPL-2.0

//! EROFS utilities

use std::{
    collections::{BTreeMap, HashMap},
    io::{self, BufWriter, Write},
};

use astr::AStr;
use stone::{StonePayloadLayoutFile, StonePayloadLayoutRecord};
use vfs::tree::Element;

// TODO: Configurable?
const BLOCK_SIZE_BITS: u8 = 12;
const BLOCK_SIZE: u64 = 1 << (BLOCK_SIZE_BITS as u64);
const ZERO_BLOCK: [u8; BLOCK_SIZE as usize] = [0; BLOCK_SIZE as usize];

const SUPER_BLOCK_OFFSET: u64 = 1024;
const SUPER_BLOCK_SIZE: usize = BLOCK_SIZE as usize - SUPER_BLOCK_OFFSET as usize;
const SUPER_MAGIC_V1: u32 = 0xE0F5_E1E2;

// https://erofs.docs.kernel.org/en/latest/ondisk/core_ondisk.html#superblock-checksum
const FEATURE_COMPAT_SB_CHKSUM: u32 = 0x1;

// https://erofs.docs.kernel.org/en/latest/ondisk/core_ondisk.html#inode-data-layouts
const INODE_FLAT_INLINE: u16 = 2;
const INODE_FLAT_PLAIN: u16 = 0;
// https://erofs.docs.kernel.org/en/latest/ondisk/core_ondisk.html#i-format-field
const INODE_VERSION_EXTENDED: u16 = 1;

const DIRENT_SIZE: usize = 12;
const SLOT_SIZE: usize = 32;

const ST_IFDIR: u16 = 0o040_000;
const ST_IFREG: u16 = 0o100_000;
const ST_IFLNK: u16 = 0o120_000;

/// A writer capable of producing an EROFS meta-only image from
/// a [`vfs::Tree`] of [`StonePayloadLayoutRecord`] entries.
#[derive(Debug, Clone, Copy, Default)]
pub struct MetaImageWriter {
    pub xattr_namespace: XattrNamespace,
}

impl MetaImageWriter {
    /// Returns a new, default [`MetaImageWriter`].
    pub fn new() -> Self {
        Self::default()
    }

    /// Specify which namespace the xattrs should be written to.
    ///
    /// Defaults to [`XattrNamespace::Trusted`].
    pub fn with_xattr_namespace(self, xattr_namespace: XattrNamespace) -> Self {
        Self { xattr_namespace }
    }

    /// Writes an EROFS meta-only image to the provided `writer` using
    /// the provided [`vfs::Tree`] of [`StonePayloadLayoutRecord`] entries.
    pub fn write<'a, T, W>(self, tree: &'a vfs::Tree<T>, writer: &'a mut W) -> io::Result<()>
    where
        T: AsRef<StonePayloadLayoutRecord>,
        W: Write,
    {
        write_meta_image(tree, &self.xattr_namespace, writer)
    }
}

/// The namespace that extended attributes get written to.
#[derive(Debug, Clone, Copy, Default)]
#[repr(u8)]
pub enum XattrNamespace {
    /// User extended attributes (`user.*`)
    User = 1,
    /// Trusted extended attributes (`trusted.*`)
    #[default]
    Trusted = 4,
}

fn write_meta_image<T, W>(tree: &vfs::Tree<T>, xattr_namespace: &XattrNamespace, writer: &mut W) -> io::Result<()>
where
    T: AsRef<StonePayloadLayoutRecord>,
    W: Write,
{
    // Buffer by block size
    let writer = &mut BufWriter::with_capacity(BLOCK_SIZE as usize, writer);

    // Get root element of VFS tree (/)
    let root_element = tree
        .structured()
        .ok_or_else(|| io::Error::other("vfs missing root / directory"))?;

    // Build inodes from vfs
    let mut inodes: Vec<Inode<'_>> = Vec::with_capacity(tree.len() as usize);
    build_inodes(&root_element, &mut inodes, None);

    // Compute layout
    let layout = compute_layout(&inodes)?;

    // Write all blocks

    // Write superblock
    write_padded(writer, BLOCK_SIZE as usize, |writer| write_superblock(writer, &layout))?;

    // Write shared attrs area, block aligned
    write_padded(writer, BLOCK_SIZE as usize, |writer| {
        write_shared_xattrs(writer, xattr_namespace, &layout.redirects)
    })?;

    // Write each meta block
    let mut inode_idx = 0;
    for _ in 0..layout.meta_blocks {
        write_padded(writer, BLOCK_SIZE as usize, |writer| {
            let mut cursor = 0usize;

            while inode_idx < layout.inodes.len() {
                let inode = &layout.inodes[inode_idx];
                let aligned_size = inode.aligned_size();

                if cursor + aligned_size > BLOCK_SIZE as usize {
                    break;
                }

                write_padded(writer, SLOT_SIZE, |writer| write_inode(writer, inode, &layout))?;

                cursor += aligned_size;
                inode_idx += 1;
            }

            Ok(())
        })?;
    }
    debug_assert!(inode_idx == layout.inodes.len(), "All inodes should be written");

    // Write dir blocks
    for packed_dirent in layout.packed_dirents.values() {
        for block in &packed_dirent.blocks {
            write_padded(writer, BLOCK_SIZE as usize, |writer| {
                write_dirent_block(writer, block, &layout.nid_mapping)
            })?;
        }
    }

    writer.flush()?;

    Ok(())
}

struct Dirent<'a> {
    name: &'a str,
    child_ino: u64,
    file_type: DirentFileType,
}

// https://erofs.docs.kernel.org/en/latest/ondisk/core_ondisk.html#file-type-values
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DirentFileType {
    Unknown = 0,
    RegFile = 1,
    Dir = 2,
    Chrdev = 3,
    Blkdev = 4,
    Fifo = 5,
    Sock = 6,
    Symlink = 7,
}

fn dirent_file_type(file: &StonePayloadLayoutFile) -> DirentFileType {
    match file {
        StonePayloadLayoutFile::Directory(_) => DirentFileType::Dir,
        StonePayloadLayoutFile::Regular(_, _) => DirentFileType::RegFile,
        StonePayloadLayoutFile::Symlink(_, _) => DirentFileType::Symlink,
        StonePayloadLayoutFile::CharacterDevice(..) => DirentFileType::Chrdev,
        StonePayloadLayoutFile::BlockDevice(..) => DirentFileType::Blkdev,
        StonePayloadLayoutFile::Fifo(..) => DirentFileType::Fifo,
        StonePayloadLayoutFile::Socket(..) => DirentFileType::Sock,
        StonePayloadLayoutFile::Unknown(..) => DirentFileType::Unknown,
    }
}

struct Inode<'a> {
    ino: u64,
    mode: u16,
    uid: u32,
    gid: u32,
    kind: InodeKind<'a>,
}

impl<'a> Inode<'a> {
    fn aligned_size(&self) -> usize {
        let size = match &self.kind {
            InodeKind::Dir { .. } => 64,
            InodeKind::Reg { .. } => {
                64 +
                // Inlined xattrs
                (12 + 4 * 2)
            }
            InodeKind::Symlink { source, .. } => {
                64 +
                // Inlined symlink data
                source.len()
            }
        };

        // Inodes must align to slot boundary
        let num_slots = size.div_ceil(SLOT_SIZE);

        num_slots * SLOT_SIZE
    }
}

enum InodeKind<'a> {
    Dir(InodeDir<'a>),
    Reg { cas_path: AStr },
    Symlink { source: &'a [u8] },
}

struct InodeDir<'a> {
    children: Vec<Dirent<'a>>,
    num_hardlinks: u32,
}

fn build_inodes<'a, T>(element: &'a Element<'a, T>, inodes: &mut Vec<Inode<'a>>, parent_ino: Option<u64>) -> Option<u64>
where
    T: AsRef<StonePayloadLayoutRecord>,
{
    let ino = inodes.len() as u64;
    let layout = element.item().as_ref();

    match &layout.file {
        StonePayloadLayoutFile::Directory(_) => {
            inodes.push(Inode {
                ino,
                mode: ST_IFDIR | (layout.mode & 0o7777) as u16,
                uid: layout.uid,
                gid: layout.gid,
                kind: InodeKind::Dir(InodeDir {
                    // Filled in after we collect children
                    children: vec![
                        Dirent {
                            name: ".",
                            child_ino: ino,
                            file_type: DirentFileType::Dir,
                        },
                        Dirent {
                            name: "..",
                            child_ino: parent_ino.unwrap_or(ino),
                            file_type: DirentFileType::Dir,
                        },
                    ],
                    // Filled in after adding all children
                    num_hardlinks: 0,
                }),
            });

            for child in element.children() {
                let Some(child_nid) = build_inodes(child, inodes, Some(ino)) else {
                    continue;
                };

                if let InodeKind::Dir(InodeDir { children, .. }) = &mut inodes[ino as usize].kind {
                    let layout = child.item().as_ref();
                    let name = child.file_name();
                    children.push(Dirent {
                        name,
                        child_ino: child_nid,
                        file_type: dirent_file_type(&layout.file),
                    });
                }
            }

            let inode = &mut inodes[ino as usize];

            if let InodeKind::Dir(InodeDir {
                children,
                num_hardlinks,
                ..
            }) = &mut inode.kind
            {
                // Ensure children are sorted
                children.sort_by(|a, b| a.name.as_bytes().cmp(b.name.as_bytes()));

                // ., .., and each subdirs ..
                //
                // Technically `..` points to the parent, but that
                // means the parent also links to this so it
                // contributes the same, so we can simply just
                // always take the total number of subdirs
                *num_hardlinks = children.iter().filter(|e| e.file_type == DirentFileType::Dir).count() as u32;
            }

            Some(ino)
        }
        StonePayloadLayoutFile::Regular(id, _) => {
            let cas_path = AStr::from(cas_path(id));

            inodes.push(Inode {
                ino,
                mode: ST_IFREG | (layout.mode & 0xFFFF) as u16,
                uid: layout.uid,
                gid: layout.gid,
                kind: InodeKind::Reg { cas_path },
            });

            Some(ino)
        }
        StonePayloadLayoutFile::Symlink(source, _) => {
            inodes.push(Inode {
                ino,
                mode: ST_IFLNK | (layout.mode & 0xFFFF) as u16,
                uid: layout.uid,
                gid: layout.gid,
                kind: InodeKind::Symlink {
                    source: source.as_bytes(),
                },
            });

            Some(ino)
        }
        _ => None,
    }
}

struct ComputedLayout<'a> {
    inodes: &'a [Inode<'a>],
    redirects: BTreeMap<AStr, u32>,
    nid_mapping: HashMap<u64, u64>,
    packed_dirents: BTreeMap<u64, PackedDirent<'a>>,
    xattr_blkaddr: u32,
    meta_blkaddr: u32,
    meta_blocks: u32,
    total_blocks: u32,
}

fn compute_layout<'a>(inodes: &'a [Inode<'a>]) -> io::Result<ComputedLayout<'a>> {
    let mut inode_dirs = vec![];
    let mut redirects = BTreeMap::new();
    let mut nid_mapping = HashMap::new();
    let mut packed_dirents = BTreeMap::new();
    let mut metadata_bytes = 0u64;

    // Compute inode placement (nid)
    for inode in inodes {
        // Inode metadata entries are aligned to "slots"
        let aligned_size = inode.aligned_size();

        // Skip to next block if this inode doesn't fit cleanly within a block
        if metadata_bytes % BLOCK_SIZE + aligned_size as u64 > BLOCK_SIZE {
            metadata_bytes += BLOCK_SIZE - (metadata_bytes % BLOCK_SIZE);
        }

        // Nid is the relative slot offset into the meta block
        let nid = metadata_bytes / SLOT_SIZE as u64;
        // Add to mapping table so we can reference the real NID offsets
        // when writing out direntry blocks
        nid_mapping.insert(inode.ino, nid);

        // Track total size so we know how many metablocks will be used
        metadata_bytes += aligned_size as u64;

        match &inode.kind {
            InodeKind::Dir(inode_dir) => {
                // Push each dir to a smaller collection which will
                // be used later to compute all direntry blocks
                inode_dirs.push((inode.ino, inode_dir));
            }
            InodeKind::Reg { cas_path } => {
                // Get the unique set of cas paths. We will later
                // calculate its relative offset in the shared xattr
                // block & use this map as a lookup when writing
                // this inodes metadata
                redirects.insert(cas_path.clone(), 0);
            }
            InodeKind::Symlink { .. } => {}
        }
    }

    // Compute shared xattr area size

    // We write out a metacopy entry as the first offset without
    // any hash == fs-verity is disabled. All regular files will
    // reference this single xattr.
    let mut xattr_bytes = xattr_entry_size("overlay.metacopy", b"");

    // Each unique cas has a redirect xattr. Its offset will be
    // the related inodes inlined shared pointer looked up from
    // this redirect map.
    for (cas_path, id) in redirects.iter_mut() {
        *id = (xattr_bytes / 4) as u32;
        xattr_bytes += xattr_entry_size("overlay.redirect", cas_path.as_bytes());
    }

    // Layout all block addresses

    // First block after superblock
    let xattr_blkaddr = 1u32;
    let xattr_blocks = (xattr_bytes).div_ceil(BLOCK_SIZE as usize) as u32;

    // After xattr block
    let meta_blkaddr = xattr_blkaddr + xattr_blocks;
    let meta_blocks = (metadata_bytes).div_ceil(BLOCK_SIZE) as u32;

    // After meta block
    let dirent_blkaddr = meta_blkaddr + meta_blocks;

    // Compute each direntry blkaddr
    let mut dirent_blkaddr_cursor = dirent_blkaddr;
    // Each dir gets a continuous range of packed blocks
    // to store its dir entries in
    for (ino, dir) in inode_dirs {
        // Pack dir entries into as few blocks as possible,
        // ensuring each entry falls completely within a clean block
        let packed_dirent = pack_dirent(dirent_blkaddr_cursor, &dir.children);
        let num_blocks = packed_dirent.blocks.len() as u32;

        // Next dir lands on the next sequential block
        dirent_blkaddr_cursor += num_blocks;

        // Track this dirent per inode so we can
        // reference its blkaddr in the inode metadata
        packed_dirents.insert(ino, packed_dirent);
    }

    let total_blocks = dirent_blkaddr_cursor;

    Ok(ComputedLayout {
        inodes,
        redirects,
        nid_mapping,
        packed_dirents,
        xattr_blkaddr,
        meta_blkaddr,
        meta_blocks,
        total_blocks,
    })
}

fn cas_path(id: &u128) -> String {
    let hash = format!("{id:02x}");

    if hash.len() >= 10 {
        format!("/{}/{}/{}/{hash}", &hash[..2], &hash[2..4], &hash[4..6])
    } else {
        format!("/{hash}")
    }
}

/// Header + xattr suffix string + value string, aligned to 4 bytes
fn xattr_entry_size(suffix: &str, value: &[u8]) -> usize {
    let size = 4 + suffix.len() + value.len();

    if size.is_multiple_of(4) {
        size
    } else {
        size + (4 - size % 4)
    }
}

struct PaddedAdapter<'a, T: Write> {
    inner: &'a mut T,
    written: usize,
}

impl<'a, T: Write> Write for PaddedAdapter<'a, T> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let written = self.inner.write(buf)?;
        self.written += written;
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }

    fn write_vectored(&mut self, bufs: &[io::IoSlice<'_>]) -> io::Result<usize> {
        let written = self.inner.write_vectored(bufs)?;
        self.written += written;
        Ok(written)
    }

    fn write_all(&mut self, buf: &[u8]) -> io::Result<()> {
        self.inner.write_all(buf)?;
        self.written += buf.len();
        Ok(())
    }
}

fn write_padded<W: Write>(
    writer: &mut W,
    alignment: usize,
    mut f: impl FnMut(&mut PaddedAdapter<'_, W>) -> io::Result<()>,
) -> io::Result<()> {
    let mut writer = PaddedAdapter {
        inner: writer,
        written: 0,
    };

    f(&mut writer)?;

    let written = writer.written;

    if !written.is_multiple_of(alignment) {
        let pad = alignment - written % alignment;

        writer.write_all(&ZERO_BLOCK[..pad])?;
    }

    Ok(())
}

fn write_shared_xattrs<W: Write>(
    writer: &mut W,
    namespace: &XattrNamespace,
    redirects: &BTreeMap<AStr, u32>,
) -> io::Result<()> {
    write_xattr_entry(writer, namespace, "overlay.metacopy", b"")?;

    for path in redirects.keys() {
        write_xattr_entry(writer, namespace, "overlay.redirect", path.as_bytes())?;
    }

    Ok(())
}

fn write_xattr_entry<W: Write>(
    writer: &mut W,
    namespace: &XattrNamespace,
    suffix: &str,
    value: &[u8],
) -> io::Result<()> {
    write_padded(writer, 4, |writer| {
        // https://erofs.docs.kernel.org/en/latest/ondisk/xattrs.html#xattr-entry-record
        writer.write_all(&[suffix.len() as u8, *namespace as u8])?;
        writer.write_all(&(value.len() as u16).to_le_bytes())?;
        writer.write_all(suffix.as_bytes())?;
        writer.write_all(value)
    })
}

fn write_inode<W: Write>(writer: &mut W, inode: &Inode<'_>, layout: &ComputedLayout<'_>) -> io::Result<()> {
    let (data_layout, i_u_startblk, i_xattr_shared_count, i_size, i_nlink) = match &inode.kind {
        InodeKind::Dir(InodeDir { num_hardlinks, .. }) => {
            let dirent = layout
                .packed_dirents
                .get(&inode.ino)
                .expect("inode dir must have packed dirent");

            (
                INODE_FLAT_PLAIN,
                dirent.blkaddr,
                0u8,
                dirent.blocks.len() as u64 * BLOCK_SIZE,
                *num_hardlinks,
            )
        }
        InodeKind::Reg { .. } => (
            INODE_FLAT_PLAIN,
            0,
            // 2 shared refs
            2,
            0,
            1,
        ),
        InodeKind::Symlink { source, .. } => (INODE_FLAT_INLINE, 0, 0, source.len() as u64, 1),
    };

    let i_format: u16 = (data_layout << 1) | INODE_VERSION_EXTENDED;

    let i_xattr_icount = if i_xattr_shared_count > 0 {
        i_xattr_shared_count + 1
    } else {
        0
    };

    // Extended 64 byte inode
    // https://erofs.docs.kernel.org/en/latest/ondisk/core_ondisk.html#extended-inode-64-bytes
    writer.write_all(&i_format.to_le_bytes())?;
    writer.write_all(&(i_xattr_icount as u16).to_le_bytes())?;
    writer.write_all(&inode.mode.to_le_bytes())?;
    writer.write_all(&0u16.to_le_bytes())?;
    writer.write_all(&i_size.to_le_bytes())?;
    writer.write_all(&i_u_startblk.to_le_bytes())?;
    writer.write_all(&(inode.ino as u32).to_le_bytes())?;
    writer.write_all(&inode.uid.to_le_bytes())?;
    writer.write_all(&inode.gid.to_le_bytes())?;
    writer.write_all(&0u64.to_le_bytes())?;
    writer.write_all(&0u32.to_le_bytes())?;
    writer.write_all(&i_nlink.to_le_bytes())?;
    writer.write_all(&[0u8; 16])?;

    // Inline data
    if let InodeKind::Symlink { source, .. } = &inode.kind {
        writer.write_all(source)?;
    }

    // Inline xattrs
    if let InodeKind::Reg { cas_path } = &inode.kind {
        // Header
        // https://erofs.docs.kernel.org/en/latest/ondisk/xattrs.html#inline-xattr-body-header
        writer.write_all(&0u32.to_le_bytes())?;
        writer.write_all(&[i_xattr_shared_count])?;
        writer.write_all(&[0u8; 7])?;

        // Shared xattr index values
        // Metacopy
        writer.write_all(&0u32.to_le_bytes())?;
        // Redirect
        let redirect_id = layout
            .redirects
            .get(cas_path)
            .copied()
            .expect("cas path must have redirect id");
        writer.write_all(&redirect_id.to_le_bytes())?;
    }

    Ok(())
}

struct PackedDirent<'a> {
    blkaddr: u32,
    blocks: Vec<&'a [Dirent<'a>]>,
}

fn pack_dirent<'a>(blkaddr: u32, children: &'a [Dirent<'a>]) -> PackedDirent<'a> {
    let mut start = 0;
    let mut running_size = 0;
    let mut blocks = vec![];

    for (child_i, child) in children.iter().enumerate() {
        running_size += DIRENT_SIZE + child.name.len();

        // Write if this is the last entry that will fit on a full block
        let is_full = child_i == children.len() - 1
            || running_size + DIRENT_SIZE + children[child_i + 1].name.len() > BLOCK_SIZE as usize;

        if is_full {
            blocks.push(&children[start..=child_i]);

            start = child_i + 1;
            running_size = 0;
        }
    }

    PackedDirent { blkaddr, blocks }
}

fn write_dirent_block<W: Write>(
    writer: &mut W,
    children: &[Dirent<'_>],
    nid_mapping: &HashMap<u64, u64>,
) -> io::Result<()> {
    let num_entries = children.len();

    // Record the offset each filename will be at
    let mut name_offsets: Vec<u16> = Vec::with_capacity(num_entries);
    let mut cursor = num_entries * DIRENT_SIZE;
    for entry in children {
        name_offsets.push(cursor as u16);
        cursor += entry.name.len();
    }

    // Write each record, using the offset recorded above
    for (i, entry) in children.iter().enumerate() {
        let nid = nid_mapping
            .get(&entry.child_ino)
            .copied()
            .expect("inode must have nid mapping");

        // https://erofs.docs.kernel.org/en/latest/ondisk/core_ondisk.html#directory-entry-record
        writer.write_all(&nid.to_le_bytes())?;
        writer.write_all(&name_offsets[i].to_le_bytes())?;
        writer.write_all(&[entry.file_type as u8, 0])?;
    }

    // Write each filename
    //
    // https://erofs.docs.kernel.org/en/latest/ondisk/core_ondisk.html#filename-encoding
    for entry in children {
        writer.write_all(entry.name.as_bytes())?;
    }
    Ok(())
}

#[rustfmt::skip]
fn write_superblock<W: Write>(
    writer: &mut W,
    layout: &ComputedLayout<'_>,
) -> io::Result<()> {
    let mut buf = [0u8; SUPER_BLOCK_SIZE];

    // Populate required / used fields
    // https://erofs.docs.kernel.org/en/latest/ondisk/core_ondisk.html#field-definitions
    buf[0..4].copy_from_slice(&SUPER_MAGIC_V1.to_le_bytes());                 // magic
    buf[8..12].copy_from_slice(&FEATURE_COMPAT_SB_CHKSUM.to_le_bytes());      // feature_compat
    buf[12] = BLOCK_SIZE_BITS;                                                // blkszbits
    buf[16..24].copy_from_slice(&(layout.inodes.len() as u64).to_le_bytes()); // inos
    buf[36..40].copy_from_slice(&layout.total_blocks.to_le_bytes());          // blocks
    buf[40..44].copy_from_slice(&layout.meta_blkaddr.to_le_bytes());          // meta_blkaddr
    buf[44..48].copy_from_slice(&layout.xattr_blkaddr.to_le_bytes());         // xattr_blkaddr

    // Add checksum
    let checksum = crc32c(buf.as_slice());
    buf[4..8].copy_from_slice(&checksum.to_le_bytes());

    // Ensure entire block is zeroized
    writer.write_all(&ZERO_BLOCK[..SUPER_BLOCK_OFFSET as usize])?;
    writer.write_all(buf.as_slice())
}

fn crc32c(bytes: &[u8]) -> u32 {
    use crc::{CRC_32_ISCSI, Crc};

    const CRC32C: Crc<u32> = Crc::<u32>::new(&CRC_32_ISCSI);

    CRC32C.checksum(bytes)
        // Undo XOR per:
        // https://erofs.docs.kernel.org/en/latest/ondisk/core_ondisk.html#superblock-checksum
        ^ CRC32C.algorithm.xorout
}
