
pub(crate) type EncRecursiveUnit = u64;

#[derive(Copy, Clone, Debug)]
pub(crate) struct RecursiveUnit {
    /// max u2
    pub core: u64,

    /// max u16
    pub count: u64,
    pub recursion: u64, // max u16
}

impl RecursiveUnit {
    pub const EMPTY: RecursiveUnit = RecursiveUnit { core: 0, count: 0, recursion: 0 };
}

impl From<EncRecursiveUnit> for RecursiveUnit {
    fn from(unit: EncRecursiveUnit) -> Self {
        RecursiveUnit {
            core: unit & 0b11,
            count: (unit >> 2) & 0xFF_FF,
            recursion: (unit >> 18) & 0xFF_FF,
        }
    }
}

impl Into<EncRecursiveUnit> for RecursiveUnit {
    fn into(self) -> EncRecursiveUnit {
        self.core | (self.count << 2) | (self.recursion << 18)
    }
}
