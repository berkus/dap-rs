use crate::swd::Error;
use crate::swd::{APnDP, DPRegister};

type Result<T> = core::result::Result<T, Error>;

pub trait ArmDebugInterface {
    /// Helper method over `read_inner` to retry during `AckWait`.
    fn read(&mut self, wait_retries: usize, apndp: APnDP, a: DPRegister) -> Result<u32> {
        for _ in 0..wait_retries {
            match self.read_inner(apndp, a) {
                Err(Error::AckWait) => continue,
                x => return x,
            }
        }

        Err(Error::AckWait)
    }

    /// Here the actual hardware implementation for an SWD read is made.
    fn read_inner(&mut self, apndp: APnDP, a: DPRegister) -> Result<u32>;

    /// Helper method over `write_inner` to retry during `AckWait`.
    fn write(&mut self, wait_retries: usize, apndp: APnDP, a: DPRegister, data: u32) -> Result<()> {
        for _ in 0..wait_retries {
            match self.write_inner(apndp, a, data) {
                Err(Error::AckWait) => continue,
                x => return x,
            }
        }

        Err(Error::AckWait)
    }

    /// Here the actual hardware implementation for an SWD write is made.
    fn write_inner(&mut self, apndp: APnDP, a: DPRegister, data: u32) -> Result<()>;

    /// Shorthand helper to read DP registers
    fn read_dp(&mut self, wait_retries: usize, a: DPRegister) -> Result<u32> {
        self.read(wait_retries, APnDP::DP, a)
    }

    /// Shorthand helper to write DP registers
    fn write_dp(&mut self, wait_retries: usize, a: DPRegister, data: u32) -> Result<()> {
        self.write(wait_retries, APnDP::DP, a, data)
    }

    /// Shorthand helper to read AP registers
    fn read_ap(&mut self, wait_retries: usize, a: DPRegister) -> Result<u32> {
        self.read(wait_retries, APnDP::AP, a)
    }
}
