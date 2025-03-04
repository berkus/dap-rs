use super::sm::JtagState;

#[derive(Default, Copy, Clone)]
struct Tap {
    ir_len: usize,
}

const MAX_TAPS: usize = 8; // @fixme As per ADI v5.2 A1-35 "Choices for JTAG-APs"

pub struct Taps {
    // jtag state: we control state ourselves and the probe-rs knows to restart the SM after DAP_Transfer cmd...
    jtag_state: JtagState, // for switching read-write modes
    taps: [Tap; MAX_TAPS],
    num_taps: usize,
    active_tap_index: usize,
    active_tap: TapSelect,
    dangling_read: bool,
    queued_reads: usize,
}

#[derive(Default)]
struct TapSelect {
    /// This many IR bits before selected TAP
    ir_prev: usize,
    /// This many TAPs before selected
    dr_prev: usize,
    /// IR bits in the selected TAP
    ir_len: usize,
    /// This many IR bits after the selected TAP
    ir_post: usize,
    /// This many TAPs after the selected TAP
    dr_post: usize,
}

// @todo don't use, this is for write_ir through active_tap
fn add_ones_to_end(input: &[u8], this_len: usize, shift: usize) -> Vec<u8> {
    let bytes = shift / 8;
    let mut output = input.to_vec();

    let top_bits = (1 << (this_len % 8)) - 1;
    let end = output.len() - 1;
    output[end] |= !top_bits;

    let mut pad = vec![0xff; bytes];
    output.append(&mut pad);
    output
}

impl Default for Taps {
    fn default() -> Self {
        Self {
            jtag_state: JtagState::Reset,
            taps: [Tap::default(); MAX_TAPS],
            num_taps: 0,
            active_tap_index: 0,
            active_tap: TapSelect::default(),
            dangling_read: false,
            queued_reads: 0,
        }
    }
}

impl Taps {
    pub fn setup(&mut self, chain_count: usize, ir_lens: &[u8]) {
        assert!(chain_count <= MAX_TAPS); // replace with ERROR?

        for i in 0..chain_count {
            self.taps[i].ir_len = ir_lens[i].into();
        }
        self.num_taps = chain_count;
    }

    /// Select which TAP in the scan chain to operate upon.  `ir` will be shifted into its
    /// instruction register, and the other TAPs put into bypass.
    pub fn select_tap(&mut self, tap: usize, ir: &[u8]) {
        assert!(tap <= self.num_taps); // make into an ERROR!

        // Reset JTAG sm!
        // tms_sequence([1,1,1,1,1,1], boom); // self.jtag_state = JtagState::Reset;

        // self.sm.mode_reset();
        self.active_tap_index = tap;
        self.write_ir(ir); // ??
    }

    fn write_ones(&mut self, mut bits: usize) {
        let bytes = bits / 8;
        bits %= 8;

        if bytes > 0 {
            let buf = vec![0xff; bytes];
            self.sm.write_reg(Register::Instruction, &buf, 8, false);
        }
        if bits > 0 {
            let buf = vec![(1 << bits) - 1];
            self.sm
                .write_reg(Register::Instruction, &buf, bits as u8, false);
        }
    }

    /// Shift `ir` into the instruction register of the TAP selected by `select_tap`
    pub fn write_ir(&mut self, ir: &[u8]) {
        assert!(self.active < self.taps.len());
        let this_irlen = self.taps[self.active].irlen;
        assert_eq!(ir.len(), (this_irlen + 7) / 8);

        // Put downstream taps into BYPASS
        let mut after_pad = 0;
        for t in &self.taps[self.active + 1..] {
            after_pad += t.irlen;
        }
        self.write_ones(after_pad);

        let mut pad_bits = 0;
        for t in &self.taps[0..self.active] {
            pad_bits += t.irlen;
        }
        let mut total_bits = (pad_bits + this_irlen) % 8;
        if total_bits == 0 {
            total_bits = 8;
        }
        let ir = add_ones_to_end(ir, this_irlen, pad_bits);
        self.sm
            .write_reg(Register::Instruction, &ir, total_bits as u8, true);
        self.sm.change_mode(JtagState::Idle);
    }

    /// Read the instruction register of the TAP selected by `select_tap`
    pub fn read_ir(&mut self) -> Vec<u8> {
        assert!(self.active < self.taps.len());
        let this_irlen = self.taps[self.active].irlen;
        let mut pad_bits = 0;
        for t in &self.taps[self.active + 1..] {
            pad_bits += t.irlen;
        }

        // Discard the unwanted bits
        self.sm.change_mode(JtagState::Idle);
        if pad_bits > 0 {
            self.sm.read_reg(Register::Instruction, pad_bits);
        }
        self.sm.read_reg(Register::Instruction, this_irlen)
    }

    /// Shift `dr` into the data register of the TAP selected by `select_tap`.  `bits` indicates
    /// how many bits of the final byte should be written (a value of 8 will write the entire byte)
    pub fn write_dr(&mut self, dr: &[u8], bits: usize) {
        assert!(self.active < self.taps.len());
        let this_len = (dr.len() - 1) * 8 + bits;
        let pad_bits = self.active;

        let mut total_bits = (pad_bits + this_len) % 8;
        if total_bits == 0 {
            total_bits = 8;
        }
        let dr = add_ones_to_end(dr, this_len, pad_bits);
        self.sm
            .write_reg(Register::Data, &dr, total_bits as u8, true);
        self.sm.change_mode(JtagState::Idle);
    }

    /// Shift `dr` into the data register of the TAP selected by `select_tap`.  `bits` indicates
    /// how many bits of the final byte should be written (a value of 8 will
    /// write the entire byte).  Returns the bits that were shifted out while `dr` was
    /// shifted in.
    pub fn read_write_dr(&mut self, dr: &[u8], bits: usize) -> Vec<u8> {
        assert_eq!(self.queued_reads, 0);
        self.queue_dr_read_write(dr, bits);
        let total_bits = (dr.len() - 1) * 8 + bits;
        self.finish_dr_read(total_bits)
    }

    pub fn queue_dr_read_write(&mut self, dr: &[u8], bits: usize) -> bool {
        assert!(self.active < self.taps.len());
        let this_len = (dr.len() - 1) * 8 + bits;
        let pad_bits = self.active;
        let discard_bits = self.taps.len() - self.active - 1;

        let mut total_bits = (pad_bits + this_len) % 8;
        if total_bits == 0 {
            total_bits = 8;
        }
        let dr = add_ones_to_end(dr, this_len, pad_bits);
        if discard_bits > 0 {
            if !self.sm.queue_read(Register::Data, discard_bits) {
                return false;
            }
        }
        if self
            .sm
            .queue_read_write(Register::Data, &dr, total_bits as u8, true)
        {
            self.sm.change_mode(JtagState::Idle);
            self.queued_reads += 1;
            true
        } else {
            self.sm.change_mode(JtagState::Idle);
            self.dangling_read = discard_bits > 0;
            false
        }
    }

    /// Read the data register of the TAP selected by `select_tap`.  `bits` indicates the length of
    /// the data register for the current instruction.
    pub fn read_dr(&mut self, bits: usize) -> Vec<u8> {
        assert_eq!(self.queued_reads, 0);
        self.queue_dr_read(bits);
        self.finish_dr_read(bits)
    }

    pub fn queue_dr_read(&mut self, bits: usize) -> bool {
        assert!(self.active < self.taps.len());
        let pad_bits = self.active;
        let discard_bits = self.taps.len() - self.active - 1;
        let total_bits = pad_bits + bits;

        // Discard the bypass bits
        self.sm.change_mode(JtagState::Idle);
        if discard_bits > 0 {
            if !self.sm.queue_read(Register::Data, discard_bits) {
                return false;
            }
        }
        if !self.sm.queue_read(Register::Data, total_bits) {
            self.dangling_read = discard_bits > 0;
            false
        } else {
            self.queued_reads += 1;
            true
        }
    }

    pub fn finish_dr_read(&mut self, bits: usize) -> Vec<u8> {
        assert!(self.active < self.taps.len());
        let pad_bits = self.active;
        let discard_bits = self.taps.len() - self.active - 1;
        let total_bits = pad_bits + bits;

        // Discard the bypass bits
        if discard_bits > 0 {
            self.sm.cable.finish_read(discard_bits);
        }
        let mut ret = self.sm.cable.finish_read(total_bits);

        // Remove the pad bits
        if pad_bits > 0 {
            let bytes = (bits + 7) / 8;
            // Trim off any extra bytes
            ret.resize(bytes, 0);

            // Mask off high bits
            if bits % 8 != 0 {
                ret[bytes - 1] &= (1 << (bits % 8)) - 1;
            }
        }

        // Handle the case where we were able to queue the read of the discard bits, but not of the
        // interesting data.
        self.queued_reads -= 1;
        if self.queued_reads == 0 && self.dangling_read {
            if discard_bits > 0 {
                self.sm.cable.finish_read(discard_bits);
            }
            self.dangling_read = false;
        }
        ret
    }
}
