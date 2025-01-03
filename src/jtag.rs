pub trait Jtag<DEPS>: From<DEPS> {
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

    // TODO: What is missing for the 2 other JTAG commands
}
