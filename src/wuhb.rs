use binrw::{BinWrite, binrw};
use indextree::{Arena, NodeId};
use std::{
    env,
    io::{Cursor, Write},
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
        for c in name.chars() {
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

    fn calculate_folder_metadata(&mut self) {
        let folder_ids: Vec<NodeId> = self
            .root
            .descendants(&self.arena)
            .filter(|&id| matches!(self.arena[id].get(), Entry::Folder(_)))
            .collect();

        let mut seek = 0;

        for id in folder_ids {
            let parent_id = id.parent(&self.arena);
            let sibling_id = id.preceding_siblings(&self.arena).nth(1);

            let parent = match parent_id {
                Some(id) => match self.arena[id].get_mut() {
                    Entry::Folder(f) => {
                        f.meta.child = f.meta.child.min(seek);
                        f.offset
                    }
                    _ => unreachable!(),
                },
                None => 0,
            };

            match sibling_id {
                Some(id) => match self.arena[id].get_mut() {
                    Entry::Folder(f) => {
                        f.meta.sibling = seek;
                    }
                    _ => unreachable!(),
                },
                None => (),
            };

            match self.arena[id].get_mut() {
                Entry::Folder(folder) => {
                    folder.offset = seek;
                    folder.meta.parent = parent;
                    folder.meta.name_size = folder.name.len() as u32;
                    folder.meta.name = folder.name.clone();

                    seek += folder.meta.bytes() as u32;
                    seek = seek.next_multiple_of(4);
                }
                _ => unreachable!(),
            }
        }
    }

    fn calculate_file_metadata(&mut self) {
        let file_ids: Vec<NodeId> = self
            .root
            .descendants(&self.arena)
            .filter(|&id| matches!(self.arena[id].get(), Entry::File(_)))
            .collect();

        let mut seek = 0;
        let mut partition = 0;

        for id in file_ids {
            let parent_id = id.parent(&self.arena).unwrap_or(self.root);
            // let sibling_id = id.preceding_siblings(&self.arena).nth(1);

            let (parent, sibling) = match self.arena[parent_id].get_mut() {
                Entry::Folder(f) => {
                    if seek < f.meta.file {
                        let sibling = f.meta.file;
                        f.meta.file = seek;
                        (f.offset, sibling)
                    } else {
                        (f.offset, NONE)
                    }
                }
                _ => unreachable!(),
            };

            match self.arena[id].get_mut() {
                Entry::File(f) => {
                    f.offset = seek;
                    f.meta.parent = parent;
                    f.meta.sibling = sibling;
                    f.meta.offset = partition;
                    f.meta.size = f.content.len() as u64;
                    // f.meta.hash = _;
                    f.meta.name_size = f.name.len() as u32;
                    f.meta.name = f.name.clone();

                    partition += f.meta.size;
                    partition = partition.next_multiple_of(16);

                    seek += f.meta.bytes() as u32;
                    seek = seek.next_multiple_of(4);
                }
                _ => unreachable!(),
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

pub fn from_rpx(rpx: Vec<u8>, config: super::WuhbConfig) -> anyhow::Result<Vec<u8>> {
    let mut fs = RomFs::new();

    let code = fs.add_folder(fs.root, "code");
    let meta = fs.add_folder(fs.root, "meta");

    fs.add_file(code, "root.rpx", rpx);

    fs.add_file(
        meta,
        "meta.ini",
        format!(
            "[menu]\nlongname={}\nshortname={}\nauthor={}\n",
            &config.long_name,
            &config.short_name,
            if cfg!(test) {
                // Insert the string used by wuhbtool to allow for byte comparison
                "Built with devkitPPC & wut"
            } else {
                env!("CARGO_PKG_AUTHORS")
            }
        )
        .into_bytes(),
    );

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
    for (_, file) in fs.files() {
        cursor.set_position((header.file_partition_ofs + file.meta.offset).next_multiple_of(16));

        cursor.write(&file.content).unwrap();
        assert_eq!(
            cursor.position(),
            header.file_partition_ofs + file.meta.offset + file.meta.size
        );
        cursor.set_position(cursor.position().next_multiple_of(4));
    }
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
    for (_, file) in fs.files() {
        cursor.set_position(header.file_table_ofs + file.offset as u64);
        file.meta.write(&mut cursor).unwrap();
    }
    header.file_table_size = cursor.position() - header.file_table_ofs;
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
    use rstest::rstest;
    use std::{fs, path::PathBuf};

    #[rstest]
    fn from_rpx(#[files("tests/dkp/wuhb/*.wuhb")] path: PathBuf) {
        let filename = path.file_stem().unwrap().to_str().unwrap();

        let rpx = fs::read(format!("./tests/dkp/rpx/{filename}.rpx")).unwrap();
        let wuhb = fs::read(format!("./tests/dkp/wuhb/{filename}.wuhb")).unwrap();

        let converted = super::from_rpx(
            rpx,
            WuhbConfig {
                long_name: String::from(filename),
                short_name: String::from(filename),
                ..Default::default()
            },
        )
        .unwrap();

        assert_eq!(converted, wuhb);
    }
}
