mod sm;
mod taps;

pub use sm::{JtagState, OperationState};
pub use taps::Taps;

use crate::adi::ArmDebugInterface;

pub trait Jtag<DEPS>: From<DEPS> + ArmDebugInterface {
    /// If JTAG is available or not.
    const AVAILABLE: bool;
    type Error;

    /// Handle a JTAG sequence request.
    ///
    /// Returns the number of bytes of rxbuf which were written to.
    fn sequences(&mut self, data: &[u8], rxbuf: &mut [u8]) -> u32;

    /// Set the maximum clock frequency, return `true` if it is valid.
    fn set_clock(&mut self, max_frequency: u32) -> bool;

    fn configure_taps(&mut self, req: &[u8]) -> Result<(), Self::Error>;

    /// Drive state machine to requested state by feeding in a TMS bit sequence.
    fn tms_sequence(&mut self, data: &[u8], nbits: usize);

    fn taps(&mut self) -> &mut Taps; // to access the TAP sm inside JTAG state?
}
