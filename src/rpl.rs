use crate::elf;
use binrw::{BinRead, BinWrite};
use flate2::{Crc, write::ZlibEncoder};
use std::{
    collections::HashMap,
    ffi::CStr,
    io::{Cursor, Write},
};

#[derive(Debug)]
struct Section {
    pub header: elf::SectionHeader,
    pub name: String,
    pub data: SectionData,
    pub index: usize,
}

#[derive(Debug, Clone)]
struct SectionData(Vec<u8>);

impl SectionData {
    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    // pub fn as_bytes_mut(&mut self) -> &mut [u8] {
    //     &mut self.0
    // }

    pub fn as_vec(&self) -> Vec<u8> {
        self.0.clone()
    }

    pub fn from_vec(&mut self, data: Vec<u8>) {
        self.0 = data;
    }

    pub fn as_rela(&self) -> Vec<elf::Rela> {
        Vec::read_options(
            &mut Cursor::new(&self.0),
            binrw::Endian::Big,
            binrw::args! { count: self.0.len() / size_of::<elf::Rela>() },
        )
        .unwrap()
    }

    pub fn from_rela(&mut self, rela: Vec<elf::Rela>) {
        let mut writer = Cursor::new(Vec::new());

        rela.write_options(&mut writer, binrw::Endian::Big, ())
            .unwrap();

        self.0 = writer.into_inner();
    }

    pub fn as_symbol(&self) -> Vec<elf::Symbol> {
        Vec::read_options(
            &mut Cursor::new(&self.0),
            binrw::Endian::Big,
            binrw::args! { count: self.0.len() / size_of::<elf::Symbol>() },
        )
        .unwrap()
    }

    // pub fn from_symbol(&mut self, symbols: Vec<elf::Symbol>) {
    //     let mut writer = Cursor::new(Vec::new());

    //     symbols
    //         .write_options(&mut writer, binrw::Endian::Big, ())
    //         .unwrap();

    //     self.0 = writer.into_inner();
    // }
}

#[derive(Debug)]
struct ElfFile {
    pub header: elf::Header,
    pub sections: Vec<Section>,
    pub num_discarded_sections: usize,
}

impl ElfFile {
    const CODE_BASE_ADDRESS: u32 = 0x02000000;
    const DATA_BASE_ADDRESS: u32 = 0x10000000;
    const LOAD_BASE_ADDRESS: u32 = 0xC0000000;

    const DEFLATE_MIN_SECTION_SIZE: usize = 0x18;

    fn read(data: Vec<u8>) -> Self {
        let mut cursor = Cursor::new(&data);

        let header = elf::Header::read(&mut cursor).unwrap();

        if header.magic != elf::Magic::ELF {
            panic!("Invalid ELF magic");
        }

        if header.file_class != elf::Class::B32 {
            panic!("Invalid ELF file class");
        }

        if header.encoding != elf::Data::MSB {
            panic!("Invalid ELF endianess");
        }

        if header.machine != elf::Machine::PPC {
            panic!("Invalid ELF machine type");
        }

        if header.elf_version != 1 {
            panic!("Invalid ident version");
        }

        if header.version != 1 {
            panic!("Invalid ELF version")
        }

        cursor.set_position(header.shoff as u64);

        let mut sections = Vec::with_capacity(header.shnum as usize);
        for _ in 0..header.shnum {
            let mut section_header = elf::SectionHeader::read(&mut cursor).unwrap();

            if section_header.size > 0 && section_header.ty != elf::SectionType::NOBITS {
                if section_header.addr >= Self::DATA_BASE_ADDRESS
                    && section_header.addr < Self::LOAD_BASE_ADDRESS
                {
                    section_header.flags |= elf::SectionFlags::WRITE;
                }

                let start = section_header.offset as usize;
                let end = start + section_header.size as usize;

                sections.push(Section {
                    header: section_header,
                    data: SectionData(data[start..end].to_vec()),
                    name: String::new(),
                    index: 0,
                });
            } else {
                sections.push(Section {
                    header: section_header,
                    data: SectionData(Vec::new()),
                    name: String::new(),
                    index: 0,
                });
            }
        }

        let str_table = sections[header.shstrndx as usize].data.as_vec();

        for section in &mut sections {
            section.name = CStr::from_bytes_until_nul(&str_table[(section.header.name as usize)..])
                .unwrap()
                .to_str()
                .unwrap()
                .to_string();
        }

        let mut num_discarded_sections = 0;
        let mut valid_index = 0;

        for i in 0..sections.len() {
            let name = if sections[i].header.ty == elf::SectionType::RELA {
                sections[sections[i].header.info as usize].name.clone()
            } else {
                sections[i].name.clone()
            };

            let section = &mut sections[i];

            if name.starts_with(".debug_") {
                section.header.ty = elf::SectionType::NULL;
                section.header.addr = 0;
                section.header.offset = 0;
                section.header.size = 0;
                section.data.0.clear(); // Assuming section.data.0 based on earlier snippets
                section.index = usize::MAX;
                num_discarded_sections += 1;
            } else {
                section.index = valid_index;
                valid_index += 1;
            }
        }

        Self {
            header,
            sections,
            num_discarded_sections,
        }
    }

    fn fix_section_flags(&mut self) {
        for section in &mut self.sections {
            match section.name.as_str() {
                ".cafe_load_bounds" => section.header.flags = elf::SectionFlags::ALLOC,
                ".rodata" | ".eh_frame" => {
                    section.header.flags = elf::SectionFlags::ALLOC | elf::SectionFlags::WRITE
                }
                _ => (),
            }
        }
    }

    fn fix_section_types(&mut self) {
        for section in &mut self.sections {
            if section.name == ".fexports" {
                section.header.ty = elf::SectionType::RPL_EXPORTS;
            } else if section.name.starts_with(".dimport_") || section.name.starts_with(".fimport_")
            {
                section.header.ty = elf::SectionType::RPL_IMPORTS;
            }
        }
    }

    fn add_eh_frame_relocations(&mut self) {
        fn read_u32(data: &[u8], off: usize) -> u32 {
            u32::from_be_bytes(data[off..off + 4].try_into().unwrap())
        }

        fn skip_leb128(data: &[u8], p: &mut usize) {
            while data[*p] & 0x80 != 0 {
                *p += 1;
            }
            *p += 1;
        }

        fn is_absolute(enc: u8) -> bool {
            enc != 0xff && enc & 0xf0 == 0
        }

        fn pointer_size(enc: u8) -> usize {
            match enc & 0x0f {
                0x00 | 0x03 | 0x0b => 4,
                0x02 | 0x0a => 2,
                0x04 | 0x0c => 8,
                e => panic!("unsupported pointer encoding {e:#x}"),
            }
        }

        let Some(symtab) = self
            .sections
            .iter()
            .position(|s| s.header.ty == elf::SectionType::SYMTAB)
        else {
            return;
        };
        let symbols = self.sections[symtab].data.as_symbol();
        let strtab = self.sections[self.sections[symtab].header.link as usize]
            .data
            .as_vec();
        let name = |s: &elf::Symbol| {
            CStr::from_bytes_until_nul(&strtab[s.name as usize..])
                .ok()
                .and_then(|n| n.to_str().ok())
        };

        let Some(eh_frame) = symbols.iter().find(|s| name(s) == Some("__eh_frame")) else {
            return;
        };

        let text = self
            .sections
            .iter()
            .position(|s| s.name == ".text")
            .expect("missing .text");
        let text_addr = self.sections[text].header.addr;
        let section_symbol = symbols
            .iter()
            .position(|s| {
                elf::SymbolType(s.info & 0xF) == elf::SymbolType::SECTION
                    && s.shndx as usize == text
            })
            .expect("missing .text section symbol") as u32;
        let rela = self
            .sections
            .iter()
            .position(|s| s.header.ty == elf::SectionType::RELA && s.header.info as usize == text)
            .expect("missing .rela.text");

        let data = self.sections[text].data.as_vec();
        let reloc = |p: usize| elf::Rela {
            offset: text_addr + p as u32,
            info: (section_symbol << 8) | elf::RelaType::PPC_ADDR32.0,
            addend: (read_u32(&data, p) - text_addr) as i32,
        };

        let mut added = Vec::new();
        let mut cies = HashMap::new();
        let mut off = (eh_frame.value - text_addr) as usize;

        loop {
            let len = read_u32(&data, off) as usize;
            if len == 0 {
                break;
            }
            let id = read_u32(&data, off + 4) as usize;
            let mut p = off + 8;

            if id == 0 {
                let version = data[p];
                p += 1;
                let aug = CStr::from_bytes_until_nul(&data[p..]).unwrap().to_bytes();
                p += aug.len() + 1;
                skip_leb128(&data, &mut p); // code alignment
                skip_leb128(&data, &mut p); // data alignment
                if version == 1 {
                    p += 1;
                } else {
                    skip_leb128(&data, &mut p);
                }

                let (mut fde_enc, mut lsda_enc) = (0, 0xff);
                if aug.first() == Some(&b'z') {
                    skip_leb128(&data, &mut p);
                    for &c in &aug[1..] {
                        let enc = data[p];
                        p += 1;
                        match c {
                            b'P' => {
                                if is_absolute(enc) {
                                    added.push(reloc(p));
                                }
                                p += pointer_size(enc);
                            }
                            b'L' => lsda_enc = enc,
                            b'R' => fde_enc = enc,
                            _ => panic!("unsupported CIE augmentation {:?}", c as char),
                        }
                    }
                }
                cies.insert(off, (fde_enc, lsda_enc));
            } else {
                let (fde_enc, lsda_enc) = cies[&(off + 4 - id)];
                p += 2 * pointer_size(fde_enc);
                if lsda_enc != 0xff {
                    skip_leb128(&data, &mut p);
                    if is_absolute(lsda_enc) && read_u32(&data, p) != 0 {
                        added.push(reloc(p));
                    }
                }
            }

            off += 4 + len;
        }

        log::debug!("Added {} .eh_frame relocations", added.len());
        let mut rels = self.sections[rela].data.as_rela();
        rels.extend(added);
        rels.sort_by_key(|r| r.offset);
        self.sections[rela].data.from_rela(rels);
    }

    fn fix_relocations(&mut self) {
        for i in 0..self.sections.len() {
            if self.sections[i].header.ty != elf::SectionType::RELA {
                continue;
            }

            self.sections[i].header.flags = elf::SectionFlags::EMPTY;

            // let symbol = self.sections[i].header.link as usize;
            // let target = self.sections[i].header.info as usize;

            let mut rels = self.sections[i].data.as_rela();

            let mut j = 0;
            while j < rels.len() {
                let info = rels[j].info;
                let addend = rels[j].addend;
                let offset = rels[j].offset;
                let index = info >> 8;
                let ty = elf::RelaType(info & 0xFF);

                match ty {
                    elf::RelaType::PPC_NONE
                    | elf::RelaType::PPC_ADDR32
                    | elf::RelaType::PPC_ADDR16_LO
                    | elf::RelaType::PPC_ADDR16_HI
                    | elf::RelaType::PPC_ADDR16_HA
                    | elf::RelaType::PPC_REL24
                    | elf::RelaType::PPC_REL14
                    | elf::RelaType::PPC_DTPMOD32
                    | elf::RelaType::PPC_DTPREL32
                    | elf::RelaType::PPC_EMB_SDA21
                    | elf::RelaType::PPC_EMB_RELSDA
                    | elf::RelaType::PPC_DIAB_SDA21_LO
                    | elf::RelaType::PPC_DIAB_SDA21_HI
                    | elf::RelaType::PPC_DIAB_SDA21_HA
                    | elf::RelaType::PPC_DIAB_RELSDA_LO
                    | elf::RelaType::PPC_DIAB_RELSDA_HI
                    | elf::RelaType::PPC_DIAB_RELSDA_HA => (),
                    elf::RelaType::PPC_REL32 => {
                        // let symbols = self.sections[symbol].data.as_symbol();
                        rels[j].info = (index << 8) | elf::RelaType::PPC_GHS_REL16_HI.0;
                        rels[j].addend = addend;
                        rels[j].offset = offset;

                        j += 1;
                        rels.insert(
                            j,
                            elf::Rela {
                                info: (index << 8) | elf::RelaType::PPC_GHS_REL16_LO.0,
                                addend: addend + 2,
                                offset: offset + 2,
                            },
                        );
                    }
                    v => panic!("Unsupported relocation type: {v:?}"),
                }

                j += 1;
            }

            self.sections[i].header.size = rels.len() as u32;
            self.sections[i].data.from_rela(rels);
        }
    }

    fn fix_loader_virtual_addresses(&mut self) {
        let mut load_max = Self::LOAD_BASE_ADDRESS;
        for section in &self.sections {
            if section.header.addr >= load_max {
                load_max = section.header.addr + section.data.len() as u32;
            }
        }

        for i in 0..self.sections.len() {
            match self.sections[i].header.ty {
                elf::SectionType::SYMTAB | elf::SectionType::STRTAB => {
                    load_max = load_max.next_multiple_of(self.sections[i].header.addralign);
                    // relocate section
                    {
                        let section_size = self.sections[i].data.len() as u32;
                        let old_sec_address = (
                            self.sections[i].header.addr,
                            self.sections[i].header.addr + section_size,
                        );

                        // Relocate symbols pointing into this section
                        for j in 0..self.sections.len() {
                            if self.sections[j].header.ty != elf::SectionType::SYMTAB {
                                continue;
                            }

                            let mut symbols = self.sections[j].data.as_symbol();

                            for symbol in &mut symbols {
                                let ty = elf::SymbolType(symbol.info & 0xf);
                                let value = symbol.value;

                                match ty {
                                    elf::SymbolType::OBJECT
                                    | elf::SymbolType::FUNC
                                    | elf::SymbolType::SECTION => (),
                                    _ => {
                                        if value >= old_sec_address.0 && value <= old_sec_address.1
                                        {
                                            symbol.value = (value - old_sec_address.0) + load_max;
                                        }
                                    }
                                }
                            }
                        }

                        // Relocate relocations pointing into this section
                        for section in &mut self.sections {
                            if section.header.ty != elf::SectionType::RELA
                                || section.header.info != i as u32
                            {
                                continue;
                            }

                            let mut rels = section.data.as_rela();

                            for rela in &mut rels {
                                let offset = rela.offset;

                                if offset >= old_sec_address.0 && offset <= old_sec_address.1 {
                                    rela.offset = (offset - old_sec_address.0) + load_max;
                                }
                            }

                            section.data.from_rela(rels);
                        }

                        self.sections[i].header.addr = load_max;
                    }
                    self.sections[i].header.flags |= elf::SectionFlags::ALLOC;
                    load_max += self.sections[i].data.len() as u32;
                }
                _ => (),
            }
        }
    }

    fn generate_file_info_section(&mut self, is_rpl: bool) {
        let mut info = elf::RplFileInfo {
            version: 0xCAFE0402,
            text_size: 0,
            text_align: 32,
            data_size: 0,
            data_align: 4096,
            load_size: 0,
            load_align: 32,
            temp_size: 0,
            tramp_adjust: 0,
            tramp_addition: 0,
            sda_base: 0,
            sda2_base: 0,
            stack_size: 0x10000,
            heap_size: 0x8000,
            filename: 0,
            flags: if is_rpl { 0x0 } else { 0x2 },
            min_version: 0x5078,
            compression_level: 6,
            file_info_pad: 0,
            cafe_sdk_version: 0x5335,
            cafe_sdk_revision: 0x10D4B,
            tls_align_shift: 0,
            tls_module_index: 0,
            runtime_file_info_size: 0,
            tag_offset: 0,
        };

        for section in &self.sections {
            let mut size = section.data.len() as u32;

            if section.index == usize::MAX {
                continue;
            }

            if section.header.ty == elf::SectionType::NOBITS {
                size = section.header.size;
            }

            match section.header.addr {
                0 if section.header.ty != elf::SectionType::RPL_CRCS
                    || section.header.ty != elf::SectionType::RPL_FILEINFO =>
                {
                    info.temp_size += size + 128;
                }
                Self::CODE_BASE_ADDRESS..Self::DATA_BASE_ADDRESS => {
                    info.text_size = info
                        .text_size
                        .max(section.header.addr + section.header.size - Self::CODE_BASE_ADDRESS);
                }
                Self::DATA_BASE_ADDRESS..Self::LOAD_BASE_ADDRESS => {
                    info.data_size = info
                        .data_size
                        .max(section.header.addr + section.header.size - Self::DATA_BASE_ADDRESS);
                }
                Self::LOAD_BASE_ADDRESS.. => {
                    info.load_size = info
                        .load_size
                        .max(section.header.addr + section.header.size - Self::LOAD_BASE_ADDRESS);
                }
                _ => panic!("Invalid section address"),
            }
        }

        info.text_size = info.text_size.next_multiple_of(info.text_align);
        info.data_size = info.data_size.next_multiple_of(info.data_align);
        info.load_size = info.load_size.next_multiple_of(info.load_align);

        self.sections.push(Section {
            header: elf::SectionHeader {
                name: 0,
                ty: elf::SectionType::RPL_FILEINFO,
                flags: elf::SectionFlags::EMPTY,
                addr: 0,
                offset: 0,
                size: 0,
                link: 0,
                info: 0,
                addralign: 4,
                entsize: 0,
            },
            name: String::new(),
            data: SectionData({
                let mut writer = Cursor::new(Vec::new());
                info.write(&mut writer).unwrap();
                writer.into_inner()
            }),
            index: self.sections.len(),
        });
        self.header.shnum += 1;
    }

    fn generate_crc_section(&mut self) {
        let mut crcs = Vec::new();

        for section in &self.sections {
            let mut crc = 0;

            if section.index == usize::MAX {
                continue;
            }

            if section.data.len() > 0 {
                let mut hasher = Crc::new();
                hasher.update(section.data.as_bytes());
                crc = hasher.sum();
            }

            crcs.push(crc);
        }

        if !crcs.is_empty() {
            let last_idx = crcs.len() - 1;
            crcs.insert(last_idx, 0);
        } else {
            crcs.push(0);
        }

        self.sections.insert(
            self.sections.len() - 1,
            Section {
                header: elf::SectionHeader {
                    name: 0,
                    ty: elf::SectionType::RPL_CRCS,
                    flags: elf::SectionFlags::EMPTY,
                    addr: 0,
                    offset: 0,
                    size: 0,
                    link: 0,
                    info: 0,
                    addralign: 4,
                    entsize: 4,
                },
                name: String::new(),
                data: SectionData({
                    let mut bytes = Vec::new();
                    for crc in crcs {
                        bytes.extend_from_slice(&crc.to_be_bytes());
                    }
                    bytes
                }),
                index: self.sections.len(),
            },
        );
        self.header.shnum += 1;
    }

    fn fix_file_header(&mut self) {
        self.header.abi = elf::Eabi::CAFE;
        self.header.ty = 0xFE01;
        self.header.flags = 0;
        self.header.phoff = 0;
        self.header.phentsize = 0;
        self.header.phnum = 0;
        self.header.shoff = 64;
        self.header.shnum = (self.sections.len() - self.num_discarded_sections) as u16;
        self.header.shstrndx = self.sections[self.header.shstrndx as usize].index as u16;
    }

    fn deflate_sections(&mut self) {
        for section in &mut self.sections {
            if section.data.len() < Self::DEFLATE_MIN_SECTION_SIZE
                || section.header.ty == elf::SectionType::RPL_CRCS
                || section.header.ty == elf::SectionType::RPL_FILEINFO
            {
                continue;
            }

            // 1. Pre-allocate and insert the 4-byte uncompressed size (Big Endian)
            let size = (section.data.len() as u32).to_be_bytes();
            let mut deflated = size.to_vec();

            // 2. Compress directly into the vector (appends after the 4 bytes)
            {
                let mut encoder = ZlibEncoder::new(&mut deflated, flate2::Compression::new(6));
                encoder.write_all(section.data.as_bytes()).unwrap();
                encoder.finish().unwrap(); // Flushes and drops encoder, releasing the borrow
            }

            // 3. Update the section data
            section.data.from_vec(deflated);
            section.header.flags |= elf::SectionFlags::DEFLATED;
        }
    }

    fn calculate_section_offsets(&mut self) {
        let mut offset = self.header.shoff;

        offset += ((self.sections.len() - self.num_discarded_sections)
            * size_of::<elf::SectionHeader>())
        .next_multiple_of(64) as u32;

        for section in &mut self.sections {
            match section.header.ty {
                elf::SectionType::NOBITS | elf::SectionType::NULL => {
                    section.data.0.clear();
                }
                _ => (),
            }
            section.header.offset = 0;
        }

        for section in &mut self.sections {
            if section.header.ty == elf::SectionType::RPL_CRCS {
                section.header.offset = offset;
                section.header.size = section.data.len() as u32;
                offset += section.header.size;
            }
        }

        for section in &mut self.sections {
            if section.header.ty == elf::SectionType::RPL_FILEINFO {
                section.header.offset = offset;
                section.header.size = section.data.len() as u32;
                offset += section.header.size;
            }
        }

        // First the "dataMin / dataMax" sections, which are:
        for section in &mut self.sections {
            if section.header.size == 0
                || section.header.ty == elf::SectionType::RPL_FILEINFO
                || section.header.ty == elf::SectionType::RPL_IMPORTS
                || section.header.ty == elf::SectionType::RPL_CRCS
                || section.header.ty == elf::SectionType::NOBITS
            {
                continue;
            }

            if (section.header.flags.0 & elf::SectionFlags::EXEC.0 == 0)
                && section.header.flags.0 & elf::SectionFlags::WRITE.0 != 0
                && section.header.flags.0 & elf::SectionFlags::ALLOC.0 != 0
            {
                section.header.offset = offset;
                section.header.size = section.data.len() as u32;
                offset += section.header.size;
            }
        }

        // Next the "readMin / readMax" sections, which are:
        for section in &mut self.sections {
            if section.header.size > 0 && section.header.flags.0 & elf::SectionFlags::ALLOC.0 != 0 {
                if section.header.ty == elf::SectionType::RPL_EXPORTS
                    || section.header.ty == elf::SectionType::RPL_IMPORTS
                    || section.header.flags.0
                        & (elf::SectionFlags::EXEC.0 | elf::SectionFlags::WRITE.0)
                        == 0
                {
                    section.header.offset = offset;
                    section.header.size = section.data.len() as u32;
                    offset += section.header.size;
                }
            }
        }

        // Next the "textMin / textMax" sections, which are:
        for section in &mut self.sections {
            if section.header.size == 0
                || section.header.ty == elf::SectionType::RPL_FILEINFO
                || section.header.ty == elf::SectionType::RPL_IMPORTS
                || section.header.ty == elf::SectionType::RPL_CRCS
                || section.header.ty == elf::SectionType::NOBITS
            {
                continue;
            }

            if section.header.flags.0 & elf::SectionFlags::EXEC.0 != 0
                && section.header.ty != elf::SectionType::RPL_EXPORTS
            {
                section.header.offset = offset;
                section.header.size = section.data.len() as u32;
                offset += section.header.size;
            }
        }

        // Next the "tempMin / tempMax" sections, which are:
        for section in &mut self.sections {
            if section.header.size == 0
                || section.header.ty == elf::SectionType::RPL_FILEINFO
                || section.header.ty == elf::SectionType::RPL_IMPORTS
                || section.header.ty == elf::SectionType::RPL_CRCS
                || section.header.ty == elf::SectionType::NOBITS
            {
                continue;
            }

            if section.header.flags.0 & elf::SectionFlags::EXEC.0 == 0
                && section.header.flags.0 & elf::SectionFlags::ALLOC.0 == 0
            {
                section.header.offset = offset;
                section.header.size = section.data.len() as u32;
                offset += section.header.size;
            }
        }

        for i in 0..self.sections.len() {
            if self.sections[i].header.offset == 0
                && self.sections[i].header.ty != elf::SectionType::NULL
                && self.sections[i].header.ty != elf::SectionType::NOBITS
            {
                panic!(
                    "Failed to calculate offset for section: {:?}",
                    self.sections[i].name
                );
            }

            if self.sections[i].index == usize::MAX {
                continue;
            }

            if self.sections[i].header.link != 0 {
                // FIXED: Using .header.link as the index, not .header.info
                self.sections[i].header.link =
                    self.sections[self.sections[i].header.link as usize].index as u32;
            }

            if self.sections[i].header.ty == elf::SectionType::RELA {
                self.sections[i].header.info =
                    self.sections[self.sections[i].header.info as usize].index as u32;
            }
        }
    }

    fn write(&self) -> Vec<u8> {
        let shoff = self.header.shoff;

        let mut cursor = Cursor::new(Vec::new());

        self.header.write(&mut cursor).unwrap();

        cursor.set_position(shoff as u64);

        for section in &self.sections {
            if section.index != usize::MAX {
                section.header.write(&mut cursor).unwrap();
            }
        }

        for section in &self.sections {
            if section.data.len() > 0 {
                cursor.set_position(section.header.offset as u64);
                cursor.write(section.data.as_bytes()).unwrap();
            }
        }

        cursor.into_inner()
    }
}

pub fn from_elf(input: Vec<u8>, is_rpl: bool) -> Vec<u8> {
    let mut elf = ElfFile::read(input);
    log::info!("Elf file parsed successfully");
    elf.fix_section_flags();
    log::debug!("Section flags fixed");
    elf.fix_section_types();
    log::debug!("Section types fixed");
    elf.add_eh_frame_relocations();
    log::debug!("Eh frame relocations added");
    elf.fix_relocations();
    log::debug!("Relocations fixed");
    elf.fix_loader_virtual_addresses();
    log::debug!("Loader virtual addresses fixed");
    elf.generate_file_info_section(is_rpl);
    log::debug!("File info section generated");
    elf.generate_crc_section();
    log::debug!("CRC section generated");
    elf.fix_file_header();
    log::debug!("File header fixed");
    elf.deflate_sections();
    log::debug!("Sections deflated");
    elf.calculate_section_offsets();
    log::debug!("Section offsets calculated");
    let output = elf.write();
    log::info!("RPL file written successfully");
    output
}

#[cfg(test)]
mod tests {
    // use rstest::rstest;
    // use std::{fs, path::PathBuf};

    // Cannot test like this because some minor differences
    // #[test]
    // fn from_elf() {
    //     let elf = fs::read("tests/dkp/elf/helloworld.elf").unwrap();
    //     let rpx = fs::read("tests/dkp/rpx/helloworld.rpx").unwrap();

    //     let converted = super::from_elf(elf, false);

    //     fs::write("tests/converted.rpx", &converted).unwrap();

    //     assert_eq!(converted, rpx);
    // }
}
