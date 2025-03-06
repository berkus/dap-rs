// This is wholly snatched from probe-rs - probably deserves it's own JTAG SM crate?

/// Inner states of the parallel arms (IR-Scan and DR-Scan) of the JTAG state machine.
#[derive(Clone, Copy, PartialEq, Debug, defmt::Format)]
pub enum OperationState {
    Select,
    Capture,
    Shift,
    Exit1,
    Pause,
    Exit2,
    Update,
}

impl OperationState {
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
    Dr(OperationState),
    Ir(OperationState),
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
            Self::Dr(OperationState::Select) => !matches!(target, Self::Dr(_)),
            Self::Ir(OperationState::Select) => !matches!(target, Self::Ir(_)),
            Self::Dr(OperationState::Update) | Self::Ir(OperationState::Update) => {
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
                    OperationState::Update
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
                    OperationState::Update
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
            Self::Idle if tms => Self::Dr(OperationState::Select),
            Self::Idle => Self::Idle,
            Self::Dr(OperationState::Select) if tms => Self::Ir(OperationState::Select),
            Self::Ir(OperationState::Select) if tms => Self::Reset,
            Self::Dr(OperationState::Update) | Self::Ir(OperationState::Update) => {
                if tms {
                    Self::Dr(OperationState::Select)
                } else {
                    Self::Idle
                }
            }
            Self::Dr(state) => Self::Dr(state.update(tms)),
            Self::Ir(state) => Self::Ir(state.update(tms)),
        };
        defmt::trace!("JTAG SM input tms {}, new state: {}", tms, *self);
    }
}
