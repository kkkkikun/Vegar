use tock_registers::register_bitfields;

register_bitfields![u64,
    TlbiVA [
        VA OFFSET(0) NUMBITS(44) [],
        TTL OFFSET(44) NUMBITS(4) [],
        ASID OFFSET(48) NUMBITS(16) [],
    ],
    TlbiVAA [
        VA OFFSET(0) NUMBITS(44) [],
        TTL OFFSET(44) NUMBITS(4) [],
    ],
    TlbiRVAA [
        BassADDR OFFSET(0) NUMBITS(37) [],
        TLL  OFFSET(37) NUMBITS(2) [],
        NUM  OFFSET(39) NUMBITS(5) [],
        SCALE OFFSET(44) NUMBITS(2) [],
        TG  OFFSET(46) NUMBITS(2) [],
    ],
    TlbiRVA [
        BassADDR OFFSET(0) NUMBITS(37) [],
        TLL  OFFSET(37) NUMBITS(2) [],
        NUM  OFFSET(39) NUMBITS(5) [],
        SCALE OFFSET(44) NUMBITS(2) [],
        TG  OFFSET(46) NUMBITS(2) [],
        ASID OFFSET(48) NUMBITS(16) [],
    ],
    TlbiASID [
        ASID OFFSET(48) NUMBITS(16) [],
    ],
];

#[inline]
pub fn tlbi(val: impl sealed::Tlbi) {
    val.tlbi();
}

mod sealed {
    pub trait Tlbi {
        fn tlbi(&self);
    }
}

macro_rules! tlbi_all {
    ($A:ident) => {
        pub struct $A;

        impl sealed::Tlbi for $A {
            #[inline(always)]
            fn tlbi(&self) {
                match () {
                    #[cfg(target_arch = "aarch64")]
                    () => unsafe {
                        core::arch::asm!(concat!("tlbi ", stringify!($A)), options(nostack))
                    },

                    #[cfg(not(target_arch = "aarch64"))]
                    () => unimplemented!(),
                }
            }
        }
    };
}

tlbi_all!(ALLE1);
tlbi_all!(ALLE2);
tlbi_all!(ALLE3);

tlbi_all!(ALLE1IS);
// tlbi_all!(ALLE1OS);

tlbi_all!(ALLE2IS);
// tlbi_all!(ALLE2OS);

tlbi_all!(ALLE3IS);
// tlbi_all!(ALLE3OS);

tlbi_all!(VMALLE1);
tlbi_all!(VMALLE1IS);
// tlbi_all!(VMALLE1OS);

#[inline]
fn va_to_tlbi_va(va: usize) -> u64 {
    const VA_MASK: u64 = (1 << 44) - 1; // VA[55:12] => bits[43:0]Add commentMore actions
    (va as u64 >> 12) & VA_MASK
}

macro_rules! tlbi_va {
    ($A:ident) => {
        pub struct $A(u64);

        impl $A {
            #[inline]
            pub fn new(asid: usize, va: usize) -> Self {
                Self((TlbiVA::VA.val(va_to_tlbi_va(va)) +
                    TlbiVA::ASID.val(asid as u64)).value)
            }
        }

        impl sealed::Tlbi for $A {
            #[inline(always)]
            fn tlbi(&self) {
                match () {
                    #[cfg(target_arch = "aarch64")]
                    () => unsafe {
                        core::arch::asm!(concat!("tlbi ", stringify!($A), ", {}"), in(reg) self.0, options(nostack))
                    },

                    #[cfg(not(target_arch = "aarch64"))]
                    () => unimplemented!(),
                }
            }
        }
    };
}

tlbi_va!(VAE1);
tlbi_va!(VAE2);
tlbi_va!(VAE3);

tlbi_va!(VAE1IS);
// tlbi_va!(VAE1OS);

tlbi_va!(VAE2IS);
// tlbi_va!(VAE2OS);

tlbi_va!(VAE3IS);
// tlbi_va!(VAE3OS);

macro_rules! tlbi_asid {
    ($A:ident) => {
        pub struct $A(u64);

        impl $A {
            #[inline]
            pub fn new(asid: usize) -> Self {
                Self(TlbiASID::ASID.val(asid as u64).value)
            }
        }

        impl sealed::Tlbi for $A {
            #[inline(always)]
            fn tlbi(&self) {
                match () {
                    #[cfg(target_arch = "aarch64")]
                    () => unsafe {
                        core::arch::asm!(concat!("tlbi ", stringify!($A), ", {}"), in(reg) self.0, options(nostack))
                    },

                    #[cfg(not(target_arch = "aarch64"))]
                    () => unimplemented!(),
                }
            }
        }
    };
}

tlbi_asid!(ASIDE1);
tlbi_asid!(ASIDE1IS);
// tlbi_asid!(ASIDE1OS);

macro_rules! tlbi_vaa {
    ($A:ident) => {
        pub struct $A(u64);

        impl $A {
            #[inline]
            pub fn new(va: usize) -> Self {
                Self(TlbiVAA::VA.val(va_to_tlbi_va(va)).value)
            }
        }

        impl sealed::Tlbi for $A {
            #[inline(always)]
            fn tlbi(&self) {
                match () {
                    #[cfg(target_arch = "aarch64")]
                    () => unsafe {
                        core::arch::asm!(concat!("tlbi ", stringify!($A), ", {}"), in(reg) self.0, options(nostack))
                    },

                    #[cfg(not(target_arch = "aarch64"))]
                    () => unimplemented!(),
                }
            }
        }
    };
}

tlbi_vaa!(VAAE1);
tlbi_vaa!(VAAE1IS);
// tlbi_vaa!(VAAE1OS);
