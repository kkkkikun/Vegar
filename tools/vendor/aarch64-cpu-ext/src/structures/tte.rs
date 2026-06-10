use core::marker::PhantomData;

/// This module defines the Translation Table Entry (TTE) structure used in AArch64 architecture.
use tock_registers::{LocalRegisterCopy, register_bitfields};

pub trait Granule: Clone + Copy {
    const M: u32;
    const SIZE: usize = 2usize.pow(Self::M);
    const MASK: u64 = (1u64 << Self::M) - 1; // Mask for alignment
}

#[derive(Clone, Copy)]
pub struct Granule4KB {}

impl Granule for Granule4KB {
    const M: u32 = 12; // log2(4096) = 12
}

#[derive(Clone, Copy)]
pub struct Granule16KB {}

impl Granule for Granule16KB {
    const M: u32 = 14; // log2(16384) = 14
}

#[derive(Clone, Copy)]
pub struct Granule64KB {}

impl Granule for Granule64KB {
    const M: u32 = 16; // log2(65536) = 16
}

pub trait OA: Clone + Copy {
    const BITS: usize;
}

#[derive(Clone, Copy)]
pub struct OA48 {}

impl OA for OA48 {
    const BITS: usize = 48; // 48-bit output address
}

#[derive(Clone, Copy)]
pub struct OA52 {}

impl OA for OA52 {
    const BITS: usize = 52; // 52-bit output address
}

/// Access permissions for Stage 1 translation using Direct permissions
/// Based on ARM DDI 0487K.a Table D8-49
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AccessPermission {
    /// Read/write access for privileged level only, no access for unprivileged
    /// AP[2:1] = 0b00 (when supporting two privilege levels)
    /// For single privilege level: Read/write access
    PrivilegedReadWrite = 0b00,

    /// Read/write access for both privileged and unprivileged levels
    /// AP[2:1] = 0b01
    ReadWrite = 0b01,

    /// Read-only access for privileged level only, no access for unprivileged
    /// AP[2:1] = 0b10 (when supporting two privilege levels)
    /// For single privilege level: Read-only access
    PrivilegedReadOnly = 0b10,

    /// Read-only access for both privileged and unprivileged levels
    /// AP[2:1] = 0b11
    ReadOnly = 0b11,
}

impl AccessPermission {
    /// Get the AP field value for the TTE
    pub const fn as_bits(self) -> u8 {
        self as u8
    }

    /// Create from AP bits
    pub const fn from_bits(bits: u8) -> Option<Self> {
        match bits & 0b11 {
            0b00 => Some(Self::PrivilegedReadWrite),
            0b01 => Some(Self::ReadWrite),
            0b10 => Some(Self::PrivilegedReadOnly),
            0b11 => Some(Self::ReadOnly),
            _ => None,
        }
    }

    /// Check if this permission allows unprivileged access
    pub const fn allows_unprivileged(self) -> bool {
        matches!(self, Self::ReadWrite | Self::ReadOnly)
    }

    /// Check if this permission allows write access at the privileged level
    pub const fn allows_privileged_write(self) -> bool {
        matches!(self, Self::PrivilegedReadWrite | Self::ReadWrite)
    }

    /// Check if this permission allows write access at the unprivileged level
    pub const fn allows_unprivileged_write(self) -> bool {
        matches!(self, Self::ReadWrite)
    }
}

/// Shareability
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Shareability {
    NonShareable,
    OuterShareable,
    InnerShareable,
}

register_bitfields![u64,
    /// Translation Table Entry for AArch64
    /// Based on ARMv8-A Architecture Reference Manual
    TTE64_REG [
        /// Valid bit - indicates if this entry is valid
        VALID OFFSET(0) NUMBITS(1) [
            Invalid = 0,
            Valid = 1
        ],

        /// Type bit for level 0, 1, and 2 entries
        /// Combined with VALID bit determines the entry type
        TYPE OFFSET(1) NUMBITS(1) [
            Block = 0,  // Block entry (when VALID=1)
            Table = 1   // Table entry (when VALID=1)
        ],

        /// Memory attributes index for MAIR_ELx
        ATTR_INDX OFFSET(2) NUMBITS(3) [],

        /// Non-secure bit
        NS OFFSET(5) NUMBITS(1) [
            Secure = 0,
            NonSecure = 1
        ],

        /// Access permission bits
        /// AP[2:1] for Stage 1 translation using Direct permissions
        /// Based on ARM DDI 0487K.a Table D8-49
        AP OFFSET(6) NUMBITS(2) [
            PrivilegedReadWrite = 0b00,  // Read/write for privileged level only
            ReadWrite = 0b01,            // Read/write for both privileged and unprivileged
            PrivilegedReadOnly = 0b10,   // Read-only for privileged level only
            ReadOnly = 0b11              // Read-only for both privileged and unprivileged
        ],

        /// Shareability field
        SH OFFSET(8) NUMBITS(2) [
            NonShareable = 0b00,
            OuterShareable = 0b10,
            InnerShareable = 0b11
        ],

        /// Access flag
        AF OFFSET(10) NUMBITS(1) [
            NotAccessed = 0,
            Accessed = 1
        ],

        /// Not global bit
        NG OFFSET(11) NUMBITS(1) [
            Global = 0,
            NotGlobal = 1
        ],

        ADDR OFFSET(12) NUMBITS(38) [],

        /// Dirty bit modifier (ARMv8.1+)
        DBM OFFSET(51) NUMBITS(1) [
            ReadOnly = 0,
            Writable = 1
        ],

        /// Contiguous bit
        CONTIG OFFSET(52) NUMBITS(1) [
            NotContiguous = 0,
            Contiguous = 1
        ],

        /// Privileged execute-never
        PXN OFFSET(53) NUMBITS(1) [
            ExecuteAllowed = 0,
            ExecuteNever = 1
        ],

        /// Execute-never or Unprivileged execute-never
        XN_UXN OFFSET(54) NUMBITS(1) [
            ExecuteAllowed = 0,
            ExecuteNever = 1
        ],

        /// Reserved for software use (bits 58:55)
        SW_RESERVED OFFSET(55) NUMBITS(4) []
    ]
];

#[derive(Clone, Copy)]
pub struct TTE64<G: Granule, O: OA> {
    reg: LocalRegisterCopy<u64, TTE64_REG::Register>,
    _marker: PhantomData<(G, O)>,
}

impl<G: Granule, O: OA> TTE64<G, O> {
    /// Create a new TTE64 from a raw u64 value
    pub const fn new(value: u64) -> Self {
        Self {
            reg: LocalRegisterCopy::new(value),
            _marker: PhantomData,
        }
    }

    /// Create an invalid TTE (all zeros)
    pub const fn invalid() -> Self {
        Self::new(0)
    }

    /// Create a table entry with more convenient parameters
    pub fn new_table(table_addr: u64) -> Self {
        let mut tte = Self::new(0);

        tte.reg
            .modify(TTE64_REG::VALID::Valid + TTE64_REG::TYPE::Table + TTE64_REG::AF::Accessed);
        tte.set_address(table_addr);
        tte
    }

    /// Create a block entry with BlockConfig
    pub fn new_block(block_addr: u64) -> Self {
        let mut tte = Self::new(0);

        tte.reg
            .modify(TTE64_REG::VALID::Valid + TTE64_REG::TYPE::Block + TTE64_REG::AF::Accessed);
        tte.set_address(block_addr);
        tte
    }

    /// Get the raw u64 value
    pub fn get(&self) -> u64 {
        self.reg.get()
    }

    /// Check if this TTE is valid
    pub fn is_valid(&self) -> bool {
        self.reg.is_set(TTE64_REG::VALID)
    }

    pub fn set_is_valid(&mut self, val: bool) {
        if val {
            self.reg.modify(TTE64_REG::VALID::Valid);
        } else {
            self.reg.modify(TTE64_REG::VALID::Invalid);
        }
    }

    /// Check if this TTE is a table entry (vs block entry)
    pub fn is_table(&self) -> bool {
        self.is_valid() && self.reg.is_set(TTE64_REG::TYPE)
    }

    /// Check if this TTE is a block entry (vs table entry)
    pub fn is_block(&self) -> bool {
        self.is_valid() && !self.reg.is_set(TTE64_REG::TYPE)
    }

    pub fn set_is_table(&mut self) {
        self.reg.modify(TTE64_REG::TYPE::Table);
    }

    pub fn set_is_block(&mut self) {
        self.reg.modify(TTE64_REG::TYPE::Block);
    }

    pub fn set_address(&mut self, addr: u64) {
        assert!(
            addr & G::MASK == 0,
            "Address must be aligned to granule size"
        );
        assert!(
            addr < (1u64 << O::BITS),
            "Address exceeds output address width"
        );
        let val = addr >> TTE64_REG::ADDR.shift; // Shift to align with TTE address bits
        self.reg.modify(TTE64_REG::ADDR.val(val));
    }

    /// Get the output address (physical address) from this TTE
    /// This extracts the address bits and reconstructs the physical address
    pub fn address(&self) -> u64 {
        if !self.is_valid() {
            return 0;
        }

        let raw_value = self.reg.get();
        let m = G::M; // granule size log2 (12, 14, or 16)

        let bit_start = m;
        let bit_end =

        // Handle 52-bit output address extension
        if O::BITS == 52 && (G::M == 12 || G::M == 14) {
            50
        } else {
            48
        };
        let mask = ((1u64 << (bit_end - bit_start + 1)) - 1) << bit_start;
        raw_value & mask
    }

    pub fn address_with_page_level(&self, level: usize) -> u64 {
        if self.is_table() {
            return self.address();
        }
        let raw_addr = self.reg.get();
        let n = match (G::M, level) {
            (12, 0) => 39,
            (12, 1) => 30,
            (12, 2) => 21,
            (14, 1) => 36,
            (14, 2) => 25,
            (16, 1) => 42,
            (16, 2) => 29,
            _ => panic!("Invalid granule size or level combination"),
        };

        let bit_start = n;
        // 4KB and 16KB granules, 52-bit OA
        let bit_end = if O::BITS == 52 && (G::M == 12 || G::M == 14) {
            50
        } else {
            48
        };
        let mask = ((1u64 << (bit_end - bit_start + 1)) - 1) << bit_start;
        raw_addr & mask
    }

    /// Check if this TTE has the access flag set
    pub fn is_accessed(&self) -> bool {
        self.reg.is_set(TTE64_REG::AF)
    }

    /// Get the memory attribute index
    pub fn attr_index(&self) -> u64 {
        self.reg.read(TTE64_REG::ATTR_INDX)
    }

    pub fn set_attr_index(&mut self, index: u64) {
        assert!(index < 8, "Attribute index must be less than 8");
        self.reg.modify(TTE64_REG::ATTR_INDX.val(index));
    }

    /// Check if this TTE allows execution (reads XN/UXN bit at [54])
    ///
    /// Returns `true` if execution is allowed, `false` if Execute Never is set.
    /// The specific meaning depends on the translation regime - see `set_executable()` for details.
    pub fn is_executable(&self) -> bool {
        !self.reg.is_set(TTE64_REG::XN_UXN)
    }

    /// Set the execution permission (controls XN/UXN bit at [54])
    ///
    /// The meaning of this bit depends on the translation regime:
    /// - **Single privilege level**: Execute-never (XN) - controls execution for the single privilege level
    /// - **Two privilege levels**: Unprivileged Execute-never (UXN) - controls execution at unprivileged level
    /// - **EL1&0 regime with HCR_EL2.{NV, NV1} = {1, 1}**: Functions as Privileged Execute-never (PXN) when UXN effective value is 0
    ///
    /// When `val` is:
    /// - `true`: Allows execution (XN/UXN = 0)
    /// - `false`: Blocks execution (XN/UXN = 1, Execute Never)
    ///
    /// See ARM DDI 0487K.a "Stage 1 instruction execution using Direct permissions"
    pub fn set_executable(&mut self, val: bool) {
        if val {
            self.reg.modify(TTE64_REG::XN_UXN::ExecuteAllowed);
        } else {
            self.reg.modify(TTE64_REG::XN_UXN::ExecuteNever);
        }
    }

    /// Check if this TTE allows privileged execution (reads PXN bit at [53])
    ///
    /// Returns `true` if privileged execution is allowed, `false` if Privileged Execute Never is set.
    /// The specific meaning depends on the translation regime - see `set_privileged_executable()` for details.
    pub fn is_privileged_executable(&self) -> bool {
        !self.reg.is_set(TTE64_REG::PXN)
    }

    /// Set the privileged execution permission (controls PXN bit at [53])
    ///
    /// The meaning of this bit depends on the translation regime:
    /// - **Single privilege level**: RES0 (Reserved, should be 0)
    /// - **Two privilege levels**: Privileged Execute-never (PXN) - controls execution at privileged level
    /// - **EL1&0 regime with HCR_EL2.{NV, NV1} = {1, 1}**: RES0 (Reserved, should be 0)
    ///
    /// When `val` is:
    /// - `true`: Allows privileged execution (PXN = 0) - only valid for two privilege level regimes
    /// - `false`: Blocks privileged execution (PXN = 1, Execute Never) - only valid for two privilege level regimes
    ///
    /// **Note**: In single privilege level regimes or specific nested virtualization configurations,
    /// this bit is reserved and should be set to 0.
    ///
    /// See ARM DDI 0487K.a "Stage 1 instruction execution using Direct permissions"
    pub fn set_privileged_executable(&mut self, val: bool) {
        if val {
            self.reg.modify(TTE64_REG::PXN::ExecuteAllowed);
        } else {
            self.reg.modify(TTE64_REG::PXN::ExecuteNever);
        }
    }

    /// Get access permissions
    pub fn access_permission(&self) -> AccessPermission {
        AccessPermission::from_bits(self.reg.read(TTE64_REG::AP) as _).unwrap()
    }

    pub fn set_access_permission(&mut self, permission: AccessPermission) {
        self.reg
            .modify(TTE64_REG::AP.val(permission.as_bits() as u64));
    }

    /// Get shareability attributes
    pub fn shareability(&self) -> Shareability {
        match self.reg.read_as_enum(TTE64_REG::SH) {
            Some(TTE64_REG::SH::Value::NonShareable) => Shareability::NonShareable,
            Some(TTE64_REG::SH::Value::OuterShareable) => Shareability::OuterShareable,
            Some(TTE64_REG::SH::Value::InnerShareable) => Shareability::InnerShareable,
            None => unreachable!("invalid value"),
        }
    }

    pub fn set_shareability(&mut self, shareability: Shareability) {
        self.reg.modify(match shareability {
            Shareability::NonShareable => TTE64_REG::SH::NonShareable,
            Shareability::OuterShareable => TTE64_REG::SH::OuterShareable,
            Shareability::InnerShareable => TTE64_REG::SH::InnerShareable,
        });
    }

    /// Set the access flag
    pub fn set_access(&mut self) {
        self.reg.modify(TTE64_REG::AF::Accessed);
    }

    /// Clear the access flag
    pub fn clear_access(&mut self) {
        self.reg.modify(TTE64_REG::AF::NotAccessed);
    }

    /// Check if the contiguous bit is set
    pub fn is_contiguous(&self) -> bool {
        self.reg.is_set(TTE64_REG::CONTIG)
    }

    /// Set the contiguous bit
    pub fn set_contiguous(&mut self) {
        self.reg.modify(TTE64_REG::CONTIG::Contiguous);
    }

    /// Check if this is a global mapping
    pub fn is_global(&self) -> bool {
        !self.reg.is_set(TTE64_REG::NG)
    }

    /// Set the not-global bit (make it process-specific)
    pub fn set_not_global(&mut self) {
        self.reg.modify(TTE64_REG::NG::NotGlobal);
    }

    /// Check if dirty bit modifier is set (ARMv8.1+)
    pub fn is_dirty_writable(&self) -> bool {
        self.reg.is_set(TTE64_REG::DBM)
    }

    /// Get the software reserved bits
    pub fn sw_reserved(&self) -> u64 {
        self.reg.read(TTE64_REG::SW_RESERVED)
    }

    /// Set the software reserved bits
    pub fn set_sw_reserved(&mut self, value: u64) {
        self.reg.modify(TTE64_REG::SW_RESERVED.val(value & 0xF));
    }
}

// Convenient type aliases for common configurations
/// TTE with 4KB granule and 48-bit output addresses
pub type TTE4K48 = TTE64<Granule4KB, OA48>;

/// TTE with 4KB granule and 52-bit output addresses
pub type TTE4K52 = TTE64<Granule4KB, OA52>;

/// TTE with 16KB granule and 48-bit output addresses
pub type TTE16K48 = TTE64<Granule16KB, OA48>;

/// TTE with 16KB granule and 52-bit output addresses
pub type TTE16K52 = TTE64<Granule16KB, OA52>;

/// TTE with 64KB granule and 48-bit output addresses
pub type TTE64K48 = TTE64<Granule64KB, OA48>;

/// TTE with 64KB granule and 52-bit output addresses
pub type TTE64K52 = TTE64<Granule64KB, OA52>;

/// Constants for different granule sizes block sizes at different levels
pub mod block_sizes {
    /// Block sizes for 4KB granule
    pub mod granule_4k {
        pub const LEVEL1_BLOCK_SIZE: usize = 1024 * 1024 * 1024; // 1GB
        pub const LEVEL2_BLOCK_SIZE: usize = 2 * 1024 * 1024; // 2MB
        pub const LEVEL3_PAGE_SIZE: usize = 4 * 1024; // 4KB
    }

    /// Block sizes for 16KB granule
    pub mod granule_16k {
        pub const LEVEL1_BLOCK_SIZE: usize = 64 * 1024 * 1024 * 1024; // 64GB
        pub const LEVEL2_BLOCK_SIZE: usize = 32 * 1024 * 1024; // 32MB
        pub const LEVEL3_PAGE_SIZE: usize = 16 * 1024; // 16KB
    }

    /// Block sizes for 64KB granule
    pub mod granule_64k {
        pub const LEVEL1_BLOCK_SIZE: usize = 4 * 1024 * 1024 * 1024; // 4TB (level0)
        pub const LEVEL2_BLOCK_SIZE: usize = 512 * 1024 * 1024; // 512MB
        pub const LEVEL3_PAGE_SIZE: usize = 64 * 1024; // 64KB
    }
}

/// Helper functions for address calculations
impl<G: Granule, O: OA> TTE64<G, O> {
    /// Calculate the index for a virtual address at a given level
    pub fn calculate_index(va: u64, level: usize) -> usize {
        match (G::M, level) {
            // 4KB granule
            (12, 0) => ((va >> 39) & 0x1FF) as usize, // 9 bits
            (12, 1) => ((va >> 30) & 0x1FF) as usize, // 9 bits
            (12, 2) => ((va >> 21) & 0x1FF) as usize, // 9 bits
            (12, 3) => ((va >> 12) & 0x1FF) as usize, // 9 bits
            // 16KB granule
            (14, 0) => ((va >> 47) & 0x1) as usize,   // 1 bit
            (14, 1) => ((va >> 36) & 0x7FF) as usize, // 11 bits
            (14, 2) => ((va >> 25) & 0x7FF) as usize, // 11 bits
            (14, 3) => ((va >> 14) & 0x7FF) as usize, // 11 bits
            // 64KB granule
            (16, 1) => ((va >> 42) & 0x3F) as usize, // 6 bits
            (16, 2) => ((va >> 29) & 0x1FFF) as usize, // 13 bits
            (16, 3) => ((va >> 16) & 0x1FFF) as usize, // 13 bits
            _ => panic!("Invalid granule size or level combination"),
        }
    }

    /// Check if an address is aligned to the granule boundary
    pub fn is_aligned(addr: u64) -> bool {
        (addr & G::MASK) == 0
    }

    /// Align an address down to the granule boundary
    pub fn align_down(addr: u64) -> u64 {
        addr & !G::MASK
    }

    /// Align an address up to the granule boundary
    pub fn align_up(addr: u64) -> u64 {
        (addr + G::MASK) & !G::MASK
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_address_extraction_4k_48bit() {
        // Test 4KB granule with 48-bit output address
        type TTE = TTE64<Granule4KB, OA48>;

        // Test table descriptor
        let table_addr = 0x1000_0000_1000; // 48-bit address aligned to 4KB
        let tte_table = TTE::new_table(table_addr);
        assert_eq!(tte_table.address(), table_addr);

        // Test block descriptor
        let block = 2 * 1024 * 1024; // 2MB
        let block_addr = 0x2000_0000_1000 + block; // 48-bit address aligned to 4KB 
        let tte_block = TTE::new_block(block_addr);
        assert_eq!(
            tte_block.address_with_page_level(2),
            0x2000_0000_0000 + block
        );
    }

    #[test]
    fn test_address_extraction_4k_52bit() {
        // Test 4KB granule with 52-bit output address
        type TTE = TTE64<Granule4KB, OA52>;

        let table_addr = (1 << 50) - 0x1000; // 52-bit address with high bits
        let tte_table = TTE::new_table(table_addr);
        let read_addr = tte_table.address();
        assert_eq!(
            read_addr, table_addr,
            "want {:#x} != read {:#x} address mismatch",
            table_addr, read_addr
        );
    }

    #[test]
    fn test_address_extraction_16k_48bit() {
        // Test 16KB granule with 48-bit output address
        type TTE = TTE64<Granule16KB, OA48>;

        // Test table descriptor - must be aligned to 16KB boundary
        let table_addr = (1 << 47) + 16 * 1024; // 48-bit address aligned to 16KB
        let tte_table = TTE::new_table(table_addr);
        let read = tte_table.address();
        assert_eq!(
            table_addr, read,
            "want {:#x} != read {:#x} address mismatch",
            table_addr, read
        );

        // Test block descriptor
        let block_addr = 0x2000_0000_0000; // 48-bit address aligned to 16KB
        let tte_block = TTE::new_block(block_addr);
        assert_eq!(tte_block.address(), block_addr);
    }

    #[test]
    fn test_address_extraction_16k_52bit() {
        // Test 16KB granule with 52-bit output address
        type TTE = TTE64<Granule16KB, OA52>;

        // Test with high address bits
        let table_addr = (1 << 50) - 0x4000; // 52-bit address with high bits, aligned to 16KB
        let tte_table = TTE::new_table(table_addr); // Base address aligned to 16KB

        assert_eq!(tte_table.address(), table_addr);
    }

    #[test]
    fn test_address_extraction_64k_48bit() {
        // Test 64KB granule with 48-bit output address
        type TTE = TTE64<Granule64KB, OA48>;

        // Test table descriptor - must be aligned to 64KB boundary
        let table_addr = 0x1000_0001_0000; // 48-bit address aligned to 64KB
        let tte_table = TTE::new_table(table_addr);
        assert_eq!(tte_table.address(), table_addr);

        // Test block descriptor
        let block_addr = 0x2000_0002_0000; // 48-bit address aligned to 64KB
        let tte_block = TTE::new_block(block_addr);
        assert_eq!(tte_block.address(), block_addr);
    }

    #[test]
    fn test_address_extraction_64k_52bit() {
        // Test 64KB granule with 52-bit output address
        type TTE = TTE64<Granule64KB, OA52>;

        let table_addr = 0xf00_1001_0000u64; // 52-bit address with high bits, aligned to 64KB
        let tte_table = TTE::new_table(table_addr); // Base address aligned to 64KB

        assert_eq!(
            table_addr,
            tte_table.address(),
            "want {:#x} != read {:#x} address mismatch",
            table_addr,
            tte_table.address()
        );
    }

    #[test]
    fn test_invalid_tte_address() {
        // Test that invalid TTEs return 0 address
        type TTE = TTE64<Granule4KB, OA48>;

        let tte_invalid = TTE::invalid();
        assert_eq!(tte_invalid.address(), 0);
        assert!(!tte_invalid.is_valid());
    }

    #[test]
    fn test_granule_constants() {
        // Test that granule constants match expected values for m calculation
        assert_eq!(Granule4KB::M, 12); // log2(4096) = 12
        assert_eq!(Granule16KB::M, 14); // log2(16384) = 14  
        assert_eq!(Granule64KB::M, 16); // log2(65536) = 16

        // Test that granule sizes are correct
        assert_eq!(Granule4KB::SIZE, 4096);
        assert_eq!(Granule16KB::SIZE, 16384);
        assert_eq!(Granule64KB::SIZE, 65536);

        // Test that masks are correct for alignment
        assert_eq!(Granule4KB::MASK, 0xFFF);
        assert_eq!(Granule16KB::MASK, 0x3FFF);
        assert_eq!(Granule64KB::MASK, 0xFFFF);
    }
}
