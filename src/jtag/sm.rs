// This is wholly snatched from probe-rs - probably deserves it's own JTAG crate?

/// Inner states of the parallel arms (IR-Scan and DR-Scan) of the JTAG state machine.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum RegisterState {
    Select,
    Capture,
    Shift,
    Exit1,
    Pause,
    Exit2,
    Update,
}

impl RegisterState {
    fn step_toward(self, target: Self) -> bool {
        match self {
            Self::Select => false,
            Self::Capture if matches!(target, Self::Shift) => false,
            Self::Exit1 if matches!(target, Self::Pause | Self::Exit2) => false,
            Self::Exit2 if matches!(target, Self::Shift | Self::Exit1 | Self::Pause) => false,
            Self::Update => {
                unreachable!("This is a bug, this case should have been handled by JtagState.")
            }
            _ => true,
        }
    }

    fn update(self, tms: bool) -> Self {
        if tms {
            match self {
                Self::Capture | Self::Shift => Self::Exit1,
                Self::Exit1 | Self::Exit2 => Self::Update,
                Self::Pause => Self::Exit2,
                Self::Select | Self::Update => {
                    unreachable!("This is a bug, this case should have been handled by JtagState.")
                }
            }
        } else {
            match self {
                Self::Select => Self::Capture,
                Self::Capture | Self::Shift => Self::Shift,
                Self::Exit1 | Self::Pause => Self::Pause,
                Self::Exit2 => Self::Shift,
                Self::Update => {
                    unreachable!("This is a bug, this case should have been handled by JtagState.")
                }
            }
        }
    }
}

/// JTAG State Machine representation.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum JtagState {
    Reset,
    Idle,
    Dr(RegisterState),
    Ir(RegisterState),
}

impl JtagState {
    /// Returns the TMS value that takes a step from the current state toward the target state.
    ///
    /// Returns `None` if the state machine is already in the target state.
    pub fn step_toward(self, target: Self) -> Option<bool> {
        let tms = match self {
            state if target == state => return None,
            Self::Reset => false,
            Self::Idle => true,
            Self::Dr(RegisterState::Select) => !matches!(target, Self::Dr(_)),
            Self::Ir(RegisterState::Select) => !matches!(target, Self::Ir(_)),
            Self::Dr(RegisterState::Update) | Self::Ir(RegisterState::Update) => {
                matches!(target, Self::Ir(_) | Self::Dr(_))
            }
            Self::Dr(state) => {
                // Decide if we need to stay in the current arm or not.
                // The inner state machine will handle the case where we need to loop back
                // through Run-Test/Idle.
                let next = if let Self::Dr(target) = target {
                    target
                } else {
                    // Let's aim for the inner state that can exit the scan arm.
                    RegisterState::Update
                };
                state.step_toward(next)
            }
            Self::Ir(state) => {
                // Decide if we need to stay in the current arm or not.
                // The inner state machine will handle the case where we need to loop back
                // through Run-Test/Idle.
                let next = if let Self::Ir(target) = target {
                    target
                } else {
                    // Let's aim for the inner state that can exit the scan arm.
                    RegisterState::Update
                };
                state.step_toward(next)
            }
        };
        Some(tms)
    }

    /// Updates the state machine from the given TMS bit.
    pub fn update(&mut self, tms: bool) {
        *self = match *self {
            Self::Reset if tms => Self::Reset,
            Self::Reset => Self::Idle,
            Self::Idle if tms => Self::Dr(RegisterState::Select),
            Self::Idle => Self::Idle,
            Self::Dr(RegisterState::Select) if tms => Self::Ir(RegisterState::Select),
            Self::Ir(RegisterState::Select) if tms => Self::Reset,
            Self::Dr(RegisterState::Update) | Self::Ir(RegisterState::Update) => {
                if tms {
                    Self::Dr(RegisterState::Select)
                } else {
                    Self::Idle
                }
            }
            Self::Dr(state) => Self::Dr(state.update(tms)),
            Self::Ir(state) => Self::Ir(state.update(tms)),
        };
    }
}

//=== Read/write operations on the JtagState machine ===

/// Read `bits` from the data register
pub fn read_dr(&mut self, bits: usize) -> Vec<u8> {
    self.change_mode(JtagState::ShiftDR);
    self.cable.read_data(bits)
}

/// Read `bits` from  the instruction register
pub fn read_ir(&mut self, bits: usize) -> Vec<u8> {
    self.change_mode(JtagState::ShiftIR);
    self.cable.read_data(bits)
}

/// Read `bits` from the data register
pub fn queue_read_dr(&mut self, bits: usize) -> bool {
    self.change_mode(JtagState::ShiftDR);
    self.cable.queue_read(bits)
}

/// Read `bits` from the instruction register
pub fn queue_read_ir(&mut self, reg: Register, bits: usize) -> bool {
    self.change_mode(JtagState::ShiftIR);
    self.cable.queue_read(bits)
}

/// Write `data` into the data register.  `bits` indicates how many bits
/// of the last byte should be written (8 indicates that the entire byte should be written).
/// The mode will either be ShiftIR / ShiftDR if `pause_after` is false, or PauseIR / PauseDR
/// if `pause_after` is true.  This allows for setting the register with multiple calls to
/// `write_reg`, which may be more convenient than manual bit-shifting.
pub fn write_dr(&mut self, data: &[u8], bits: u8, pause_after: bool) {
    self.change_mode(JtagState::ShiftDR);
    self.cable.write_data(data, bits, pause_after);
    if pause_after {
        self.state = JtagState::PauseDR;
    }
}

/// Write `data` into the instruction register.  `bits` indicates how many bits
/// of the last byte should be written (8 indicates that the entire byte should be written).
/// The mode will either be ShiftIR / ShiftDR if `pause_after` is false, or PauseIR / PauseDR
/// if `pause_after` is true.  This allows for setting the register with multiple calls to
/// `write_reg`, which may be more convenient than manual bit-shifting.
pub fn write_ir(&mut self, data: &[u8], bits: u8, pause_after: bool) {
    self.change_mode(JtagState::ShiftIR);
    self.cable.write_data(data, bits, pause_after);
    if pause_after {
        self.state = JtagState::PauseIR;
    }
}

/// Write `data` into either the instruction or data register. `bits` indicates how many bits
/// of the last byte should be written (8 indicates that the entire byte should be written).
/// The mode will either be ShiftIR / ShiftDR if `pause_after` is false, or PauseIR / PauseDR
/// if `pause_after` is true.  This allows for setting the register with multiple calls to
/// `read_write_reg`, which may be more convenient than manual bit-shifting.
///
/// Similar to `write_reg` except it returns the bits that were shifted out during writing.
pub fn read_write_reg(
    &mut self,
    reg: Register,
    data: &[u8],
    bits: u8,
    pause_after: bool,
) -> Vec<u8> {
    if reg == Register::Data {
        self.change_mode(JtagState::ShiftDR);
    } else {
        self.change_mode(JtagState::ShiftIR);
    }
    let data = self.cable.read_write_data(data, bits, pause_after);
    if pause_after {
        if reg == Register::Data {
            self.state = JtagState::PauseDR;
        } else {
            self.state = JtagState::PauseIR;
        }
    }
    data
}

pub fn queue_read_write(
    &mut self,
    reg: Register,
    data: &[u8],
    bits: u8,
    pause_after: bool,
) -> bool {
    if reg == Register::Data {
        self.change_mode(JtagState::ShiftDR);
    } else {
        self.change_mode(JtagState::ShiftIR);
    }
    let data = self.cable.queue_read_write(data, bits, pause_after);
    if pause_after {
        if reg == Register::Data {
            self.state = JtagState::PauseDR;
        } else {
            self.state = JtagState::PauseIR;
        }
    }
    data
}
