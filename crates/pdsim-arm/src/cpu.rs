//! The CPU the game runs on, behind [`pdsim_cpu::Cpu`].

pub use pdsim_cpu::*;

#[cfg(not(any(feature = "unicorn", feature = "thumb")))]
compile_error!("pdsim-arm needs a CPU backend: enable feature `unicorn` or `thumb`");

/// Which CPU emulator runs the game.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "unicorn", derive(Default))]
pub enum Backend {
    /// Unicorn (QEMU's TCG).
    #[cfg(feature = "unicorn")]
    #[default]
    Unicorn,
    /// The pure-Rust Thumb interpreter (`pdsim-thumb`).
    #[cfg(feature = "thumb")]
    Thumb,
}

impl Backend {
    /// A new CPU of this kind with nothing mapped.
    pub fn build<S: 'static>(self) -> Result<Box<dyn Cpu<S>>, CpuError> {
        match self {
            #[cfg(feature = "unicorn")]
            Backend::Unicorn => Ok(Box::new(crate::unicorn_cpu::UnicornCpu::new()?)),
            #[cfg(feature = "thumb")]
            Backend::Thumb => Ok(Box::new(pdsim_thumb::ThumbCpu::new())),
        }
    }
}
