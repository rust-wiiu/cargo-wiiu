use binrw::binrw;
// use bitflags::bitflags;

macro_rules! typed_value {
    ($name:ident, $inner:ty, $($constant:ident = $value:expr),* $(,)?) => {
        #[binrw]
        #[brw(big)]
        #[repr(transparent)]
        #[derive(PartialEq, Clone, Copy)]
        pub struct $name(pub $inner);

        impl $name {
            $(
                pub const $constant: Self = Self($value);
            )*
        }

        impl std::fmt::Debug for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                if self.0 == 0 {
                    return write!(f, "NULL ({:#X})", self.0);
                }
                $(
                    if self.0 == $value {
                        return write!(f, "{} ({:#X})", stringify!($constant), self.0);
                    }
                )*
                write!(f, "({:#X})", self.0)
            }
        }
    };
}

macro_rules! typed_flag {
    ($name:ident, $inner:ty, $($constant:ident = $value:expr),* $(,)?) => {
        #[binrw]
        #[brw(big)]
        #[repr(transparent)]
        #[derive(PartialEq, Eq, Clone, Copy)]
        pub struct $name(pub $inner);

        impl $name {
            $(
                pub const $constant: Self = Self($value);
            )*
        }

        impl std::ops::BitOr for $name {
            type Output = Self;
            fn bitor(self, rhs: Self) -> Self { Self(self.0 | rhs.0) }
        }

        impl std::ops::BitOrAssign for $name {
            fn bitor_assign(&mut self, rhs: Self) { self.0 |= rhs.0; }
        }

        impl std::ops::BitAnd for $name {
            type Output = Self;
            fn bitand(self, rhs: Self) -> Self { Self(self.0 & rhs.0) }
        }

        impl std::ops::BitAndAssign for $name {
            fn bitand_assign(&mut self, rhs: Self) { self.0 &= rhs.0; }
        }

        impl std::fmt::Debug for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                if self.0 == 0 {
                    return write!(f, "NULL ({:#X})", self.0);
                }

                let mut active = Vec::new();
                $(
                    if $value != 0 && (self.0 & $value) == $value {
                        active.push(stringify!($constant));
                    }
                )*

                if active.is_empty() {
                    write!(f, "({:#X})", self.0)
                } else {
                    write!(f, "{} ({:#X})", active.join(" | "), self.0)
                }
            }
        }
    };
}

typed_value!(Magic, u32, ELF = 0x7f454c46);

typed_value!(Class, u8, B32 = 1);

typed_value!(Data, u8, MSB = 2);

typed_value!(Eabi, u16, CAFE = 0xcafe);

typed_value!(Machine, u16, PPC = 0x14);

#[binrw]
#[brw(big)]
#[derive(Debug, Clone)]
pub struct Header {
    /// File identification.
    pub magic: Magic,
    /// File class.
    pub file_class: Class,
    /// Data encoding.
    pub encoding: Data,
    /// File version.
    pub elf_version: u8,
    /// OS/ABI identification. (EABI_*)
    pub abi: Eabi,
    pub pad: [u8; 7],
    /// Type of file (ET_*)
    pub ty: u16,
    /// Required architecture for this file (EM_*)
    pub machine: Machine,
    /// Must be equal to 1
    pub version: u32,
    /// Address to jump to in order to start program
    pub entry: u32,
    /// Program header table's file offset, in bytes
    pub phoff: u32,
    /// Section header table's file offset, in bytes
    pub shoff: u32,
    /// Processor-specific flags
    pub flags: u32,
    /// Size of ELF header, in bytes
    pub ehsize: u16,
    /// Size of an entry in the program header table
    pub phentsize: u16,
    /// Number of entries in the program header table
    pub phnum: u16,
    /// Size of an entry in the section header table
    pub shentsize: u16,
    /// Number of entries in the section header table
    pub shnum: u16,
    /// Sect hdr table index of sect name string table
    pub shstrndx: u16,
}

const _: () = assert!(std::mem::size_of::<Header>() == 0x34);

typed_value!(
    SectionType,
    u32,
    NULL = 0x0,
    // PROGBITS = 0x1,
    SYMTAB = 0x2,
    STRTAB = 0x3,
    RELA = 0x4,
    NOBITS = 0x8,
    RPL_EXPORTS = 0x80000001,
    RPL_IMPORTS = 0x80000002,
    RPL_CRCS = 0x80000003,
    RPL_FILEINFO = 0x80000004,
);

typed_flag!(
    SectionFlags,
    u32,
    EMPTY = 0x0,
    WRITE = 0x1,
    ALLOC = 0x2,
    EXEC = 0x4,
    DEFLATED = 0x08000000
);

#[binrw]
#[brw(big)]
#[derive(Debug, Clone)]
pub struct SectionHeader {
    /// Section name (index into string table)
    pub name: u32,
    /// Section type (SHT_*)
    pub ty: SectionType,
    /// Section flags (SHF_*)
    pub flags: SectionFlags,
    /// Address where section is to be loaded
    pub addr: u32,
    /// File offset of section data, in bytes
    pub offset: u32,
    /// Size of section, in bytes
    pub size: u32,
    /// Section type-specific header table index link
    pub link: u32,
    /// Section type-specific extra information
    pub info: u32,
    /// Section address alignment
    pub addralign: u32,
    /// Size of records contained within the section
    pub entsize: u32,
}

const _: () = assert!(std::mem::size_of::<SectionHeader>() == 0x28);

typed_value!(
    SymbolType,
    u8,
    // NOTYPE = 0x0,
    OBJECT = 0x1,
    FUNC = 0x2,
    SECTION = 0x3,
    // FILE = 0x4,
    // COMMON = 0x5,
    // TLS = 0x6,
    // LOOS = 0x7,
    // HIOS = 0x8,
    // GNU_IFUNC = 0xa,
    // LOPROC = 0xd,
    // HIPROC = 0xf,
);

#[binrw]
#[brw(big)]
#[derive(Debug, Clone)]
pub struct Symbol {
    /// Symbol name (index into string table)
    pub name: u32,
    /// Value or address associated with the symbol
    pub value: u32,
    /// Size of the symbol
    pub size: u32,
    /// Symbol's type and binding attributes
    pub info: u8,
    /// Must be zero; reserved
    pub other: u8,
    /// Which section (header table index) it's defined in (SHN_*)
    pub shndx: u16,
}

const _: () = assert!(std::mem::size_of::<Symbol>() == 0x10);

typed_value!(
    RelaType,
    u32,
    PPC_NONE = 0x0,
    PPC_ADDR32 = 0x1,
    // PPC_ADDR24 = 0x2,
    // PPC_ADDR16 = 0x3,
    PPC_ADDR16_LO = 0x4,
    PPC_ADDR16_HI = 0x5,
    PPC_ADDR16_HA = 0x6,
    // PPC_ADDR14 = 0x7,
    // PPC_ADDR14_BRTAKEN = 0x8,
    // PPC_ADDR14_BRNTAKEN = 0x9,
    PPC_REL24 = 0xa,
    PPC_REL14 = 0xb,
    // PPC_REL14_BRTAKEN = 0xc,
    // PPC_REL14_BRNTAKEN = 0xd,
    // PPC_GOT16 = 0xe,
    // PPC_GOT16_LO = 0xf,
    // PPC_GOT16_HI = 0x10,
    // PPC_GOT16_HA = 0x11,
    // PPC_PLTREL24 = 0x12,
    // PPC_JMP_SLOT = 0x15,
    // PPC_RELATIVE = 0x16,
    // PPC_LOCAL24PC = 0x17,
    PPC_REL32 = 0x1a,
    // PPC_TLS = 0x43,
    PPC_DTPMOD32 = 0x44,
    // PPC_TPREL16 = 0x45,
    // PPC_TPREL16_LO = 0x46,
    // PPC_TPREL16_HI = 0x47,
    // PPC_TPREL16_HA = 0x48,
    // PPC_TPREL32 = 0x49,
    // PPC_DTPREL16 = 0x4a,
    // PPC_DTPREL16_LO = 0x4b,
    // PPC_DTPREL16_HI = 0x4c,
    // PPC_DTPREL16_HA = 0x4d,
    PPC_DTPREL32 = 0x4e,
    // PPC_GOT_TLSGD16 = 0x4f,
    // PPC_GOT_TLSGD16_LO = 0x50,
    // PPC_GOT_TLSGD16_HI = 0x51,
    // PPC_GOT_TLSGD16_HA = 0x52,
    // PPC_GOT_TLSLD16 = 0x53,
    // PPC_GOT_TLSLD16_LO = 0x54,
    // PPC_GOT_TLSLD16_HI = 0x55,
    // PPC_GOT_TLSLD16_HA = 0x56,
    // PPC_GOT_TPREL16 = 0x57,
    // PPC_GOT_TPREL16_LO = 0x58,
    // PPC_GOT_TPREL16_HI = 0x59,
    // PPC_GOT_TPREL16_HA = 0x5a,
    // PPC_GOT_DTPREL16 = 0x5b,
    // PPC_GOT_DTPREL16_LO = 0x5c,
    // PPC_GOT_DTPREL16_HI = 0x5d,
    // PPC_GOT_DTPREL16_HA = 0x5e,
    // PPC_TLSGD = 0x5f,
    // PPC_TLSLD = 0x60,
    PPC_EMB_SDA21 = 0x6d,
    PPC_EMB_RELSDA = 0x74,
    PPC_DIAB_SDA21_LO = 0xb4,
    PPC_DIAB_SDA21_HI = 0xb5,
    PPC_DIAB_SDA21_HA = 0xb6,
    PPC_DIAB_RELSDA_LO = 0xb7,
    PPC_DIAB_RELSDA_HI = 0xb8,
    PPC_DIAB_RELSDA_HA = 0xb9,
    // PPC_GHS_REL16_HA = 0xfb,
    PPC_GHS_REL16_HI = 0xfc,
    // PPC_GHS_REL16_LO = 0xfd,
);

#[binrw]
#[brw(big)]
#[derive(Debug, Clone)]
pub struct Rela {
    pub offset: u32,
    pub info: u32,
    pub addend: i32,
}

const _: () = assert!(std::mem::size_of::<Rela>() == 0x0C);

#[binrw]
#[brw(big)]
#[derive(Debug, Clone)]
pub struct RplCrc {
    pub crc: u32,
}

const _: () = assert!(std::mem::size_of::<RplCrc>() == 0x04);

#[binrw]
#[brw(big)]
#[derive(Debug, Clone)]
pub struct RplFileInfo {
    pub version: u32,
    pub text_size: u32,
    pub text_align: u32,
    pub data_size: u32,
    pub data_align: u32,
    pub load_size: u32,
    pub load_align: u32,
    pub temp_size: u32,
    pub tramp_adjust: u32,
    pub sda_base: u32,
    pub sda2_base: u32,
    pub stack_size: u32,
    pub filename: u32,
    pub flags: u32,
    pub heap_size: u32,
    pub tag_offset: u32,
    pub min_version: u32,
    pub compression_level: i32,
    pub tramp_addition: u32,
    pub file_info_pad: u32,
    pub cafe_sdk_version: u32,
    pub cafe_sdk_revision: u32,
    pub tls_module_index: u16,
    pub tls_align_shift: u16,
    pub runtime_file_info_size: u32,
}

const _: () = assert!(std::mem::size_of::<RplFileInfo>() == 0x60);
