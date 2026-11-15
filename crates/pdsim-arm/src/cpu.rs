//! The CPU the game runs on, behind [`pdsim_cpu::Cpu`].

pub use pdsim_cpu::*;

/// Which CPU emulator runs the game.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "unicorn", derive(Default))]
pub enum Backend {
    /// Unicorn (QEMU's TCG).
    #[cfg(feature = "unicorn")]
    #[default]
    Unicorn,
}

impl Backend {
    /// A new CPU of this kind with nothing mapped.
    pub fn build<S: 'static>(self) -> Result<Box<dyn Cpu<S>>, CpuError> {
        match self {
            #[cfg(feature = "unicorn")]
            Backend::Unicorn => Ok(Box::new(crate::unicorn_cpu::UnicornCpu::new()?)),
        }
    }
}
