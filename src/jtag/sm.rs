// This is wholly snatched from probe-rs - probably deserves it's own JTAG SM crate?

/// Inner states of the parallel arms (IR-Scan and DR-Scan) of the JTAG state machine.
#[derive(Clone, Copy, PartialEq, Debug, defmt::Format)]
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
    /// Returns the TMS value that takes a step from the current state toward the target state.
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

    /// Step the state machine to a new state based on the TMS input value.
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
#[derive(Clone, Copy, PartialEq, Debug, Default, defmt::Format)]
pub enum JtagState {
    #[default]
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
        defmt::trace!("JTAG SM state: {}", *self);
    }
}

// State machine drives only TMS, so the corresponding trait method is Jtag::tms_sequence()
pub fn drive_state<DEPS>(
    jtag: &mut impl super::Jtag<DEPS>,
    mut current: JtagState,
    target: JtagState,
) {
    loop {
        let tms = current.step_toward(target);
        if tms == None {
            return;
        }
        let tms = tms.unwrap();
        current.update(tms);

        let data = if tms { [1u8; 1] } else { [0u8; 1] };

        // feed tms to tms_sequence bit-by-bit
        jtag.tms_sequence(&data, 1);
    }
}
