use std::fs::File;
use std::io::{self, BufRead, BufReader};
use std::path::Path;

const BASE_PREDICTOR_SIZE: usize = 4096;
const SMART_PREDICTOR_AMOUNT: usize = 6;
const SMART_PREDICTOR_FIRST_SIZE: usize = 5;
const SMART_PREDICTOR_TABLE_SIZE: usize = 2048;
const MAX_HISTORY_LENGTH: usize = 256;
const MAX_VERDICT_VALUE: usize = 3;
const AMOUNT_OF_FILES: usize = 1;

#[derive(Debug)]
struct TraceLine {
    pc: u64,
    taken: bool,
}

impl TraceLine {
    fn parse(line: &str) -> Self {
        let mut parts = line.split_whitespace();

        let pc_str = parts.next().expect("trace: pc not a number");
        let pc_str = pc_str.trim_start_matches("0x");

        let taken_str = parts.next().expect("trace: taken not a bool");
        let pc = u64::from_str_radix(pc_str, 16).expect("pc is a bad number");
        let taken = taken_str == "1";

        TraceLine { pc, taken }
    }
}
#[derive(Clone, Copy)]
struct SmartPredictionEntry {
    usefulness: u8,
    verdict: u8,
    tag: u16,
}

impl SmartPredictionEntry {
    fn new() -> Self {
        Self {
            usefulness: 0,
            verdict: 0b10,
            tag: 0,
        }
    }
    fn decide(&self) -> bool {
        return self.verdict > 1;
    }
    fn update(&mut self, taken: bool) {
        if taken {
            self.verdict =
                Utils::bounded_increment(self.verdict as u16, MAX_VERDICT_VALUE as u16) as u8;
        } else {
            self.verdict = Utils::bounded_decrement(self.verdict as u16) as u8;
        }
    }
}

struct SmartPredictor {
    perspicacity: u8,
    prediction_table: Vec<SmartPredictionEntry>,
    csr: CSR,
}

impl SmartPredictor {
    fn new(perspicacity: u8) -> Self {
        let hist_len = Utils::calculate_history_len(perspicacity);
        Self {
            prediction_table: vec![SmartPredictionEntry::new(); SMART_PREDICTOR_TABLE_SIZE],
            perspicacity,
            csr: CSR::new(perspicacity as usize, hist_len),
        }
    }
    fn get_table_index(&self, pc: usize) -> usize {
        let mut result: usize = pc ^ (self.csr.val as usize);
        result %= MAX_HISTORY_LENGTH;
        result
    }
    fn get_tag(&self, pc: usize) -> u16 {
        let mut result: usize = pc >> 2;
        result ^= (self.csr.val as usize) << 1;
        result %= 1 << self.perspicacity;
        result as u16
    }
}

struct GlobalHistoryRegister {
    history: std::collections::VecDeque<bool>,
}

impl GlobalHistoryRegister {
    fn new(max_len: usize) -> Self {
        GlobalHistoryRegister {
            history: std::collections::VecDeque::from(vec![false; max_len]),
        }
    }
    fn get_nth(&self, n: usize) -> bool {
        return self.history.get(n).copied().unwrap_or(false);
    }
    fn push(&mut self, new_bit: bool) {
        self.history.push_front(new_bit);
        self.history.pop_back();
    }
}

struct Tage {
    base_predictor: Vec<u16>,
    smart_predictors: Vec<SmartPredictor>,
    global_history_register: GlobalHistoryRegister,
}

#[derive(Clone, Copy)]
enum PredictionResultMeta {
    Base {
        t0_index: usize,
        prediction: bool,
    },
    Tagged {
        provider_table: usize,
        alt_table: Option<usize>,

        provider_predictor: SmartPredictionEntry,
        alt_predictor: Option<SmartPredictionEntry>,

        provider_index: usize,
        alt_index: Option<usize>,
    },
    None,
}

#[derive(Clone, Copy)]
struct PredictionResult {
    taken: bool,
    meta: PredictionResultMeta,
}

struct CSR {
    val: u64,
    out_bits: usize,
    hist_len: usize,
}
impl CSR {
    fn new(out_bits: usize, hist_len: usize) -> Self {
        CSR {
            val: 0,
            out_bits,
            hist_len,
        }
    }
    fn update(&mut self, new_bit: bool, old_bit: bool) {
        if self.out_bits == 0 {
            return;
        }
        let mut new_val = (self.val << 1) | (self.val >> (self.out_bits - 1));
        new_val &= (1 << self.out_bits) - 1;
        new_val ^= new_bit as u64;

        let old_pos = self.hist_len % self.out_bits;
        new_val ^= (old_bit as u64) << old_pos;

        self.val = new_val;
    }
}

struct Utils {}
impl Utils {
    const fn get_perspicacity(i: usize) -> usize {
        return SMART_PREDICTOR_FIRST_SIZE + i * i;
    }
    const fn calculate_history_len(perspicacity: u8) -> usize {
        (perspicacity as usize) * 4
    }
    const fn get_t0_index(pc: u64) -> usize {
        (pc & 0xFFF) as usize // get last 12 bit
    }
    const fn bounded_increment(val: u16, max_val: u16) -> u16 {
        let next = val.saturating_add(1);
        return if next > max_val { max_val } else { next };
    }
    const fn bounded_decrement(val: u16) -> u16 {
        return if val == 0 { 0 } else { val - 1 };
    }
}
impl Tage {
    fn new(amount: usize) -> Self {
        let mut smart_predictors: Vec<SmartPredictor> = vec![];
        smart_predictors.reserve(amount);
        for i in 1..=amount {
            smart_predictors.push(SmartPredictor::new(Utils::get_perspicacity(i) as u8));
        }
        Tage {
            base_predictor: vec![2; BASE_PREDICTOR_SIZE],
            smart_predictors,
            global_history_register: GlobalHistoryRegister::new(MAX_HISTORY_LENGTH),
        }
    }
    fn update_history(&mut self, new_bit: bool) {
        self.global_history_register.push(new_bit);
        self.smart_predictors.iter_mut().for_each(|p| {
            let last_bit = self.global_history_register.get_nth(p.csr.hist_len);
            p.csr.update(new_bit, last_bit);
        })
    }
    fn allocate_entry(&mut self, which_table: usize, trace_line: &TraceLine) {
        let pc = trace_line.pc as usize;

        let mut full_run = true;
        for table in which_table..SMART_PREDICTOR_AMOUNT {
            let predictor = &mut self.smart_predictors[table];
            let index = predictor.get_table_index(pc);

            if (predictor.prediction_table[index].usefulness == 0) {
                let tag = predictor.get_tag(trace_line.pc as usize);
                let table_entry = &mut predictor.prediction_table[index];
                table_entry.tag = tag;
                table_entry.usefulness = 0;
                table_entry.verdict = 0b10;
                full_run = false;
                break;
            }
        }
        if full_run {
            for table in which_table..SMART_PREDICTOR_AMOUNT {
                let predictor = &mut self.smart_predictors[table];
                let index = predictor.get_table_index(pc);

                let usefulness = &mut predictor.prediction_table[index].usefulness;
                *usefulness = Utils::bounded_decrement(*usefulness as u16) as u8;
            }
        }
    }
    fn predict_base(&self, trace_line: &TraceLine) -> PredictionResultMeta {
        let index = Utils::get_t0_index(trace_line.pc);

        let prediction = self.base_predictor[index];

        return PredictionResultMeta::Base {
            t0_index: index,
            prediction: prediction > 0b01,
        };
    }
    fn predicict_smart(&self, trace_line: &TraceLine) -> PredictionResultMeta {
        let mut provider_predictor: Option<SmartPredictionEntry> = None;
        let mut provider_table: usize = 0;
        let mut provider_index: usize = 0;

        let mut alt_predictor: Option<SmartPredictionEntry> = None;
        let mut alt_table: Option<usize> = None;
        let mut alt_index: Option<usize> = None;

        for (i, predictor) in self.smart_predictors.iter().enumerate().rev() {
            let table_index = predictor.get_table_index(trace_line.pc as usize);
            let table_entry = predictor.prediction_table[table_index];

            if table_entry.tag == predictor.get_tag(trace_line.pc as usize) {
                match (provider_predictor, alt_predictor) {
                    (None, _) => {
                        provider_predictor = Some(table_entry);
                        provider_table = i;
                        provider_index = table_index;
                    }
                    (Some(_), None) => {
                        alt_predictor = Some(table_entry);
                        alt_table = Some(i);
                        alt_index = Some(table_index);
                        break;
                    }
                    _ => {}
                }
            }
        }

        if let Some(provider_predictor) = provider_predictor {
            return PredictionResultMeta::Tagged {
                provider_table,
                alt_table,
                provider_predictor: provider_predictor,
                alt_predictor,
                alt_index,
                provider_index,
            };
        } else {
            return PredictionResultMeta::None;
        }
    }

    fn predict(&self, trace_line: &TraceLine) -> PredictionResult {
        let predict_smart = self.predicict_smart(trace_line);

        if let PredictionResultMeta::Tagged {
            provider_predictor: entry,
            ..
        } = predict_smart
        {
            return PredictionResult {
                taken: entry.decide(),
                meta: predict_smart,
            };
        }

        let predict_base = self.predict_base(trace_line);
        if let PredictionResultMeta::Base { prediction, .. } = predict_base {
            return PredictionResult {
                taken: prediction,
                meta: predict_base,
            };
        }

        unreachable!()
    }
    fn update(&mut self, prediction: PredictionResult, trace_line: &TraceLine) {
        match prediction.meta {
            PredictionResultMeta::Base {
                t0_index,
                prediction,
            } => {
                let counter = &mut self.base_predictor[t0_index];

                if trace_line.taken {
                    *counter = Utils::bounded_increment(*counter, 3);
                } else {
                    if *counter > 0 {
                        *counter = Utils::bounded_decrement(*counter);
                    }
                }

                if prediction != trace_line.taken {
                    self.allocate_entry(1, &trace_line);
                }
            }
            PredictionResultMeta::Tagged {
                provider_table,
                alt_table,
                alt_index,
                provider_index,
                ..
            } => {
                let alt_was_right = if let Some(alt_table) = alt_table {
                    let alt_index = alt_index.expect("alt_index must exist");
                    let alt_predictor =
                        &mut self.smart_predictors[alt_table].prediction_table[alt_index];
                    alt_predictor.decide() == trace_line.taken
                } else {
                    match self.predict_base(trace_line) {
                        PredictionResultMeta::Base { prediction, .. } => {
                            prediction == trace_line.taken
                        }
                        _ => unreachable!(),
                    }
                };
                let provider_predictor =
                    &mut self.smart_predictors[provider_table].prediction_table[provider_index];
                let provider_was_right = provider_predictor.decide() == trace_line.taken;
                provider_predictor.update(trace_line.taken);
                if provider_was_right && !alt_was_right {
                    provider_predictor.usefulness = provider_predictor.usefulness.saturating_add(1);
                }

                if !provider_was_right {
                    self.allocate_entry(
                        Utils::bounded_increment(
                            provider_table as u16,
                            SMART_PREDICTOR_AMOUNT as u16 + 1,
                        ) as usize,
                        trace_line,
                    );
                }
            }
            _ => {}
        }
        self.update_history(trace_line.taken);
    }
}

struct Stats {
    total: usize,
    correct: usize,
}

impl Stats {
    fn new() -> Self {
        Stats {
            total: 0,
            correct: 0,
        }
    }
    fn add_result(&mut self, was_is_correct: bool) {
        self.total += 1;
        self.correct += was_is_correct as usize;
    }
    fn get_result(self: &Stats) -> f32 {
        if self.total == 0 {
            return 0.0;
        }
        return (self.correct as f32) / (self.total as f32);
    }
}

fn run_trace(path: &Path, tage: &mut Tage) -> io::Result<Stats> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);

    let mut stats = Stats::new();

    for line in reader.lines() {
        let line = line?;
        let trace_line = TraceLine::parse(&line);
        let prediction = tage.predict(&trace_line);
        tage.update(prediction, &trace_line);
        stats.add_result(prediction.taken == trace_line.taken);
    }
    Ok(stats)
}

fn main() -> io::Result<()> {
    let mut tage = Tage::new(SMART_PREDICTOR_AMOUNT);

    for i in 1..=AMOUNT_OF_FILES {
        let path_str = format!("traces/trace_{i:02}");
        let path = Path::new(&path_str);

        let stats = run_trace(path, &mut tage).expect("bad trace stats");
        println!("{}", stats.get_result());
    }
    Ok(())
}
