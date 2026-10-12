use anyhow::Context;
use binrw::{BinWrite, binrw};
use flate2::write::GzEncoder;
use indextree::{Arena, NodeId};
use std::{
    io::{Cursor, Write},
    path::Path,
};

#[binrw]
#[brw(big)]
#[derive(Debug, Clone)]
struct RomFsHeader {
    magic: [u8; 4],
    size: u32,
    dir_hash_table_ofs: u64,
    dir_hash_table_size: u64,
    dir_table_ofs: u64,
    dir_table_size: u64,
    file_hash_table_ofs: u64,
    file_hash_table_size: u64,
    file_table_ofs: u64,
    file_table_size: u64,
    file_partition_ofs: u64,
}

impl Default for RomFsHeader {
    fn default() -> Self {
        Self {
            magic: *b"WUHB",
            size: 0x50,
            dir_hash_table_ofs: 0,
            dir_hash_table_size: 0,
            dir_table_ofs: 0,
            dir_table_size: 0,
            file_hash_table_ofs: 0,
            file_hash_table_size: 0,
            file_table_ofs: 0,
            file_table_size: 0,
            file_partition_ofs: 0x200,
        }
    }
}

#[binrw]
#[brw(big)]
#[derive(Debug, Clone)]
struct RomFsDirEntry {
    /// Offset of parent into directory table
    parent: u32,
    /// Next folder with the same parent
    sibling: u32,
    /// Offset into directory table of next folder in self
    child: u32,
    /// Offset into file table of next file in self
    file: u32,
    /// Offset into folder table of next folder in hash collision list
    hash: u32,
    /// Number of bytes of folder name
    name_size: u32,
    /// Folder name
    #[br(count = name_size, try_map = String::from_utf8)]
    #[bw(map = |s| s.as_bytes())]
    name: String,
}

impl Default for RomFsDirEntry {
    fn default() -> Self {
        Self {
            parent: NONE,
            sibling: NONE,
            child: NONE,
            file: NONE,
            hash: NONE,
            name_size: 0,
            name: String::new(),
        }
    }
}

impl RomFsDirEntry {
    const SIZE: u64 = 0x18;

    fn bytes(&self) -> u64 {
        Self::SIZE + self.name_size as u64
    }
}

#[binrw]
#[brw(big)]
#[derive(Debug, Clone)]
struct RomFsFileEntry {
    /// Offset of parent into directory table
    parent: u32,
    /// Offset into file table of next file in parent folder
    sibling: u32,
    /// Offset of content into partition
    offset: u64,
    /// Size of content in partition
    size: u64,
    /// Offset into file table of next file in hash collision list
    hash: u32,
    /// Number of bytes of file name
    name_size: u32,
    /// File name
    #[br(count = name_size, try_map = String::from_utf8)]
    #[bw(map = |s| s.as_bytes())]
    name: String,
}

impl Default for RomFsFileEntry {
    fn default() -> Self {
        Self {
            parent: NONE,
            sibling: NONE,
            offset: 0,
            size: 0,
            hash: NONE,
            name_size: 0,
            name: String::new(),
        }
    }
}

impl RomFsFileEntry {
    const SIZE: u64 = 0x20;

    fn bytes(&self) -> u64 {
        Self::SIZE + self.name_size as u64
    }
}

#[binrw]
#[brw(big)]
#[br(import(len: usize))]
#[derive(Debug, Clone)]
struct HashTable(#[br(count = len)] Vec<u32>);

impl HashTable {
    fn new(expected_entries: usize) -> Self {
        Self(vec![
            NONE;
            match expected_entries {
                0..=2 => 3,
                3..=18 => expected_entries | 1,
                mut count => {
                    let small_primes = [2, 3, 5, 7, 11, 13, 17];

                    while small_primes.iter().any(|&p| count % p == 0) {
                        count += 1;
                    }

                    count
                }
            }
        ])
    }

    fn hash(&mut self, parent_offset: u32, name: &str, current_offset: u32) -> u32 {
        let mut hash = parent_offset ^ 123456789;
        for c in name.bytes() {
            hash = (hash >> 5) | (hash << 27);
            hash ^= c.to_ascii_uppercase() as u32;
        }

        let bucket = (hash as usize) % self.0.len();

        let prev = self.0[bucket];
        self.0[bucket] = current_offset;

        prev
    }

    fn bytes(&self) -> u64 {
        self.0.len() as u64 * 4
    }
}

#[derive(Clone, Debug)]
struct Folder {
    name: String,
    meta: RomFsDirEntry,
    offset: u32,
}

#[derive(Clone, Debug)]
struct File {
    name: String,
    content: Vec<u8>,
    meta: RomFsFileEntry,
    offset: u32,
}

#[derive(Clone, Debug)]
enum Entry {
    Folder(Folder),
    File(File),
}

const NONE: u32 = u32::MAX;

struct RomFs {
    arena: Arena<Entry>,
    root: NodeId,
}

impl RomFs {
    fn new() -> Self {
        let mut arena = Arena::new();
        let root = arena.new_node(Entry::Folder(Folder {
            name: String::new(),
            meta: RomFsDirEntry::default(),
            offset: 0,
        }));

        Self { arena, root: root }
    }

    fn add_folder(&mut self, folder: NodeId, name: &str) -> NodeId {
        let node_id = folder.append_value(
            Entry::Folder(Folder {
                name: String::from(name),
                meta: RomFsDirEntry::default(),
                offset: 0,
            }),
            &mut self.arena,
        );
        node_id
    }

    fn add_file(&mut self, folder: NodeId, name: &str, content: Vec<u8>) -> NodeId {
        let node_id = folder.append_value(
            Entry::File(File {
                name: String::from(name),
                content,
                meta: RomFsFileEntry::default(),
                offset: 0,
            }),
            &mut self.arena,
        );
        node_id
    }

    fn folders(&mut self) -> impl Iterator<Item = (NodeId, &Folder)> {
        self.root
            .descendants(&self.arena)
            .filter_map(|node_id| match self.arena[node_id].get() {
                Entry::Folder(folder) => Some((node_id, folder)),
                _ => None,
            })
    }

    fn files(&self) -> impl Iterator<Item = (NodeId, &File)> {
        self.root
            .descendants(&self.arena)
            .filter_map(|node_id| match self.arena[node_id].get() {
                Entry::File(file) => Some((node_id, file)),
                _ => None,
            })
    }

    fn folder_mut(&mut self, id: NodeId) -> &mut Folder {
        match self.arena[id].get_mut() {
            Entry::Folder(f) => f,
            _ => unreachable!(),
        }
    }

    fn file_mut(&mut self, id: NodeId) -> &mut File {
        match self.arena[id].get_mut() {
            Entry::File(f) => f,
            _ => unreachable!(),
        }
    }

    fn child_folders(&self, id: NodeId) -> Vec<NodeId> {
        id.children(&self.arena)
            .filter(|&c| matches!(self.arena[c].get(), Entry::Folder(_)))
            .collect()
    }

    fn child_files(&self, id: NodeId) -> Vec<NodeId> {
        id.children(&self.arena)
            .filter(|&c| matches!(self.arena[c].get(), Entry::File(_)))
            .collect()
    }

    /// Order in which wuhbtool lays out file entries and data: subfolders first, then own files.
    fn file_order(&self, folder: NodeId, order: &mut Vec<NodeId>) {
        for child in self.child_folders(folder) {
            self.file_order(child, order);
        }
        order.extend(self.child_files(folder));
    }

    fn calculate_folder_metadata(&mut self) {
        let folder_ids: Vec<NodeId> = self.folders().map(|(id, _)| id).collect();

        let mut seek = 0;
        for &id in &folder_ids {
            let folder = self.folder_mut(id);
            folder.offset = seek;
            folder.meta.name_size = folder.name.len() as u32;
            folder.meta.name = folder.name.clone();

            seek += folder.meta.bytes() as u32;
            seek = seek.next_multiple_of(4);
        }

        for &id in &folder_ids {
            let offset = self.folder_mut(id).offset;
            let parent = id
                .parent(&self.arena)
                .map_or(offset, |p| self.folder_mut(p).offset);
            self.folder_mut(id).meta.parent = parent;

            let children = self.child_folders(id);
            if let Some(&first) = children.first() {
                self.folder_mut(id).meta.child = self.folder_mut(first).offset;
            }
            for pair in children.windows(2) {
                self.folder_mut(pair[0]).meta.sibling = self.folder_mut(pair[1]).offset;
            }
        }
    }

    fn calculate_file_metadata(&mut self) {
        let mut file_ids = Vec::new();
        self.file_order(self.root, &mut file_ids);

        let mut seek = 0;
        let mut partition = 0;

        for id in file_ids {
            let parent = id.parent(&self.arena).unwrap();
            let parent = self.folder_mut(parent).offset;

            let file = self.file_mut(id);
            file.offset = seek;
            file.meta.parent = parent;
            file.meta.offset = partition;
            file.meta.size = file.content.len() as u64;
            file.meta.name_size = file.name.len() as u32;
            file.meta.name = file.name.clone();

            partition += file.meta.size;
            partition = partition.next_multiple_of(16);

            seek += file.meta.bytes() as u32;
            seek = seek.next_multiple_of(4);
        }

        let folder_ids: Vec<NodeId> = self.folders().map(|(id, _)| id).collect();
        for id in folder_ids {
            let children = self.child_files(id);
            if let Some(&first) = children.first() {
                self.folder_mut(id).meta.file = self.file_mut(first).offset;
            }
            for pair in children.windows(2) {
                self.file_mut(pair[0]).meta.sibling = self.file_mut(pair[1]).offset;
            }
        }
    }

    fn calculate_hash_tables(&mut self) -> (HashTable, HashTable) {
        let mut dirs = HashTable::new(self.folders().count() + 1);
        let mut files = HashTable::new(self.files().count());

        {
            let folder_ids: Vec<NodeId> = self.folders().map(|x| x.0).collect();

            for id in folder_ids {
                let (parent_offset, name, current_offset) = match self.arena[id].get() {
                    Entry::Folder(f) => (f.meta.parent, f.name.clone(), f.offset),
                    _ => unreachable!(),
                };

                let next = dirs.hash(parent_offset, &name, current_offset);

                match self.arena[id].get_mut() {
                    Entry::Folder(f) => {
                        f.meta.hash = next;
                    }
                    _ => unreachable!(),
                }
            }

            {
                let file_ids: Vec<NodeId> = self.files().map(|x| x.0).collect();

                for id in file_ids {
                    let (parent_offset, name, current_offset) = match self.arena[id].get() {
                        Entry::File(f) => (f.meta.parent, f.name.clone(), f.offset),
                        _ => unreachable!(),
                    };

                    let next = files.hash(parent_offset, &name, current_offset);

                    match self.arena[id].get_mut() {
                        Entry::File(f) => {
                            f.meta.hash = next;
                        }
                        _ => unreachable!(),
                    }
                }
            }
        }

        (dirs, files)
    }
}

impl RomFs {
    /// Adds the contents of `path` to `folder`. Entries are sorted by name so the output is reproducible.
    fn add_dir(&mut self, folder: NodeId, path: &Path) -> anyhow::Result<()> {
        let mut entries = std::fs::read_dir(path)
            .context(format!("Failed to read directory: {}", path.display()))?
            .collect::<Result<Vec<_>, _>>()
            .context(format!("Failed to read directory: {}", path.display()))?;
        entries.sort_by_key(|e| e.file_name());

        for entry in entries {
            let path = entry.path();
            let name = entry.file_name();
            let name = name
                .to_str()
                .context(format!("File name is not valid UTF-8: {}", path.display()))?;
            let metadata = std::fs::metadata(&path)
                .context(format!("Failed to read metadata: {}", path.display()))?;

            if metadata.is_dir() {
                let child = self.add_folder(folder, name);
                self.add_dir(child, &path)?;
            } else if metadata.is_file() {
                let content = std::fs::read(&path)
                    .context(format!("Failed to read file: {}", path.display()))?;
                self.add_file(folder, name, content);
            } else {
                anyhow::bail!("Unsupported file type: {}", path.display());
            }
        }

        Ok(())
    }
}

/// Converts an image to a gzipped, uncompressed, bottom-up TGA like wuhbtool (FreeImage) does.
/// Aroma copies `iconTex.tga` verbatim into a fixed 65580 byte buffer, so the layout matters.
fn tga_gz(path: &Path, width: u32, height: u32, alpha: bool) -> anyhow::Result<Vec<u8>> {
    let image = image::open(path).context(format!("Failed to read image: {}", path.display()))?;

    anyhow::ensure!(
        image.width() == width && image.height() == height,
        "{} is {}x{}, but must be {width}x{height}",
        path.display(),
        image.width(),
        image.height()
    );

    let bpp: u8 = if alpha { 32 } else { 24 };

    let mut tga = Vec::with_capacity(18 + (width * height * bpp as u32 / 8) as usize + 26);
    tga.extend_from_slice(&[0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    tga.extend_from_slice(&(width as u16).to_le_bytes());
    tga.extend_from_slice(&(height as u16).to_le_bytes());
    tga.push(bpp);
    tga.push(if alpha { 8 } else { 0 });

    if alpha {
        for row in image.to_rgba8().rows().rev() {
            for &image::Rgba([r, g, b, a]) in row {
                tga.extend_from_slice(&[b, g, r, a]);
            }
        }
    } else {
        for row in image.to_rgb8().rows().rev() {
            for &image::Rgb([r, g, b]) in row {
                tga.extend_from_slice(&[b, g, r]);
            }
        }
    }

    tga.extend_from_slice(&[0; 8]);
    tga.extend_from_slice(b"TRUEVISION-XFILE.\0");

    let mut encoder = GzEncoder::new(Vec::new(), flate2::Compression::best());
    encoder.write_all(&tga).unwrap();
    Ok(encoder.finish().unwrap())
}

pub fn from_rpx(rpx: Vec<u8>, config: super::WuhbConfig) -> anyhow::Result<Vec<u8>> {
    let mut fs = RomFs::new();

    let code = fs.add_folder(fs.root, "code");
    let meta = fs.add_folder(fs.root, "meta");

    fs.add_file(code, "root.rpx", rpx);

    let name = &config.name;
    // Aroma stores the name in a `char[64]`
    anyhow::ensure!(
        name.len() <= 63,
        "Name must be at most 63 bytes, but is {}: {name:?}",
        name.len()
    );
    anyhow::ensure!(!name.contains('\n'), "Name must not contain a line break");

    fs.add_file(
        meta,
        "meta.ini",
        format!(
            "[menu]\nlongname={name}\nshortname={name}\nauthor={}\n",
            if cfg!(test) {
                // Insert the string used by wuhbtool to allow for byte comparison
                "Built with devkitPPC & wut"
            } else {
                &config.author
            }
        )
        .into_bytes(),
    );

    for (path, name, width, height, alpha) in [
        (&config.icon, "iconTex.tga.gz", 128, 128, true),
        (&config.tv_image, "bootTvTex.tga.gz", 1280, 720, false),
        (&config.drc_image, "bootDrcTex.tga.gz", 854, 480, false),
    ] {
        if let Some(path) = path {
            fs.add_file(meta, name, tga_gz(path, width, height, alpha)?);
        }
    }

    if let Some(path) = &config.content {
        anyhow::ensure!(
            path.is_dir(),
            "Content path is not a directory: {}",
            path.display()
        );

        // wuhbtool omits an empty content folder
        if std::fs::read_dir(path)?.next().is_some() {
            let content = fs.add_folder(fs.root, "content");
            fs.add_dir(content, path)?;
        } else {
            log::warn!("Content folder is empty: {}", path.display());
        }
    }

    log::info!("RomFS created");

    fs.calculate_folder_metadata();
    log::debug!("Folder metadata calculated");
    fs.calculate_file_metadata();
    log::debug!("File metadata calculated");
    let (dir_hash_table, file_hash_table) = fs.calculate_hash_tables();
    log::debug!("Hash tables calculated");

    let mut cursor = Cursor::new(Vec::new());

    let mut header = RomFsHeader::default();

    // File content / partition
    let mut partition_end = header.file_partition_ofs;
    for (_, file) in fs.files() {
        cursor.set_position((header.file_partition_ofs + file.meta.offset).next_multiple_of(16));

        cursor.write_all(&file.content).unwrap();
        assert_eq!(
            cursor.position(),
            header.file_partition_ofs + file.meta.offset + file.meta.size
        );
        partition_end = partition_end.max(cursor.position());
    }
    cursor.set_position(partition_end.next_multiple_of(4));
    log::debug!("File partition written");

    // Directory hash table
    header.dir_hash_table_ofs = cursor.position();
    dir_hash_table.write(&mut cursor).unwrap();
    header.dir_hash_table_size = dir_hash_table.bytes();
    log::debug!("Directory hash table written");

    // Directory entry table
    header.dir_table_ofs = cursor.position();
    for (_, folder) in fs.folders() {
        cursor.set_position(header.dir_table_ofs + folder.offset as u64);
        folder.meta.write(&mut cursor).unwrap();
    }
    // wuhbtools adds an empty entry for some reason
    {
        RomFsDirEntry {
            parent: 0,
            sibling: 0,
            child: 0,
            file: 0,
            hash: 0,
            name_size: 0,
            name: String::new(),
        }
        .write(&mut cursor)
        .unwrap();
    }
    header.dir_table_size = cursor.position() - header.dir_table_ofs;
    log::debug!("Directory table written");

    // File hash table
    header.file_hash_table_ofs = cursor.position();
    file_hash_table.write(&mut cursor).unwrap();
    header.file_hash_table_size = file_hash_table.bytes();
    log::debug!("File hash table written");

    // File entry table
    header.file_table_ofs = cursor.position();
    let mut file_table_end = header.file_table_ofs;
    for (_, file) in fs.files() {
        cursor.set_position(header.file_table_ofs + file.offset as u64);
        file.meta.write(&mut cursor).unwrap();
        file_table_end = file_table_end.max(cursor.position().next_multiple_of(4));
    }
    cursor.get_mut().resize(file_table_end as usize, 0);
    header.file_table_size = file_table_end - header.file_table_ofs;
    log::debug!("File table written");

    // File header
    cursor.set_position(0);
    header.write(&mut cursor).unwrap();
    log::debug!("File header written");

    Ok(cursor.into_inner())
}

#[cfg(test)]
mod tests {
    use crate::WuhbConfig;
    use flate2::read::GzDecoder;
    use rstest::rstest;
    use std::{fs, io::Read, path::PathBuf};

    fn decoded_tga(width: u32, height: u32, alpha: bool) -> anyhow::Result<Vec<u8>> {
        let path = std::env::temp_dir().join(format!("cargo-wiiu-{width}x{height}.png"));
        image::RgbaImage::from_fn(width, height, |x, y| {
            image::Rgba([x as u8, y as u8, 0x40, if y == 0 { 0x80 } else { 0xFF }])
        })
        .save(&path)
        .unwrap();

        let gz = super::tga_gz(&path, 128, 128, alpha);
        fs::remove_file(&path).unwrap();

        let mut tga = Vec::new();
        GzDecoder::new(gz?.as_slice())
            .read_to_end(&mut tga)
            .unwrap();
        Ok(tga)
    }

    #[test]
    fn tga_icon_layout() {
        let tga = decoded_tga(128, 128, true).unwrap();

        // Same header and size as Aroma's built-in default icon
        assert_eq!(tga.len(), 65580);
        assert_eq!(
            tga[..18],
            [
                0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x80, 0, 0x80, 0, 0x20, 0x08
            ]
        );
        assert!(tga.ends_with(b"TRUEVISION-XFILE.\0"));

        // Bottom-up rows, BGRA pixels: first pixel is (x=0, y=127)
        assert_eq!(tga[18..22], [0x40, 127, 0, 0xFF]);
        // Last row is the image's first row (alpha 0x80)
        assert_eq!(tga[18 + 127 * 128 * 4..][..4], [0x40, 0, 0, 0x80]);
    }

    #[test]
    fn tga_wrong_size() {
        let err = decoded_tga(64, 64, true).unwrap_err().to_string();
        assert!(err.contains("is 64x64, but must be 128x128"), "{err}");
    }

    /// Walks the directory and file tables via their parent/sibling/child links.
    fn list_files(wuhb: &[u8]) -> Vec<(String, Vec<u8>)> {
        let u32_at = |o: usize| u32::from_be_bytes(wuhb[o..o + 4].try_into().unwrap());
        let u64_at = |o: usize| u64::from_be_bytes(wuhb[o..o + 8].try_into().unwrap()) as usize;
        let name_at = |o: usize, len: usize| String::from_utf8(wuhb[o..o + len].to_vec()).unwrap();

        let (dir_table, file_table, partition) = (u64_at(0x18), u64_at(0x38), u64_at(0x48));

        let mut out = Vec::new();
        let mut stack = vec![(0u32, String::new())];
        while let Some((dir, path)) = stack.pop() {
            let entry = dir_table + dir as usize;
            let mut file = u32_at(entry + 12);
            while file != super::NONE {
                let f = file_table + file as usize;
                let name = name_at(f + 0x20, u32_at(f + 0x1C) as usize);
                let (offset, size) = (u64_at(f + 8), u64_at(f + 16));
                out.push((
                    format!("{path}/{name}"),
                    wuhb[partition + offset..partition + offset + size].to_vec(),
                ));
                file = u32_at(f + 4);
            }

            let mut child = u32_at(entry + 8);
            while child != super::NONE {
                let c = dir_table + child as usize;
                let name = name_at(c + 0x18, u32_at(c + 0x14) as usize);
                stack.push((child, format!("{path}/{name}")));
                child = u32_at(c + 4);
            }
        }

        out.sort();
        out
    }

    #[test]
    fn name_length() {
        let build = |name: &str| {
            super::from_rpx(
                Vec::new(),
                WuhbConfig {
                    name: String::from(name),
                    ..Default::default()
                },
            )
        };

        assert!(build(&"a".repeat(63)).is_ok());
        assert!(build(&"a".repeat(64)).is_err());
        // 32 two-byte characters are 64 bytes
        assert!(build(&"ä".repeat(32)).is_err());
    }

    #[test]
    fn content_folder() {
        let dir = std::env::temp_dir().join("cargo-wiiu-content");
        let _ = fs::remove_dir_all(&dir);

        let files: [(&str, &[u8]); 6] = [
            ("a.txt", b"root file"),
            ("assets/img/1.dat", &[1; 300]),
            ("assets/readme.md", b"nested"),
            ("empty.bin", b""),
            ("zz/sub/deep/leaf", b"leaf"),
            ("zz/Upper.TXT", b"case"),
        ];
        for (path, data) in files {
            let path = dir.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, data).unwrap();
        }
        fs::create_dir_all(dir.join("assets/empty_dir")).unwrap();

        let wuhb = super::from_rpx(
            b"rpx".to_vec(),
            WuhbConfig {
                content: Some(dir.clone()),
                ..Default::default()
            },
        );
        fs::remove_dir_all(&dir).unwrap();

        let mut expected: Vec<(String, Vec<u8>)> = files
            .iter()
            .map(|(p, d)| (format!("/content/{p}"), d.to_vec()))
            .collect();
        expected.push((String::from("/code/root.rpx"), b"rpx".to_vec()));
        expected.sort();

        let listed: Vec<_> = list_files(&wuhb.unwrap())
            .into_iter()
            .filter(|(p, _)| !p.starts_with("/meta/"))
            .collect();
        assert_eq!(listed, expected);
    }

    #[rstest]
    fn from_rpx(#[files("tests/dkp/wuhb/*.wuhb")] path: PathBuf) {
        let filename = path.file_stem().unwrap().to_str().unwrap();

        let rpx = fs::read(format!("./tests/dkp/rpx/{filename}.rpx")).unwrap();
        let wuhb = fs::read(format!("./tests/dkp/wuhb/{filename}.wuhb")).unwrap();

        let converted = super::from_rpx(
            rpx,
            WuhbConfig {
                name: String::from(filename),
                ..Default::default()
            },
        )
        .unwrap();

        assert_eq!(converted, wuhb);
    }
}
