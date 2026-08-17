use std::fs::File;
use std::io::{self, BufRead, BufReader};
use std::path::Path;

const BASE_PREDICTOR_SIZE: usize = 4096;
const SMART_PREDICTOR_AMOUNT: usize = 6;
const SMART_PREDICTOR_FIRST_SIZE: usize = 5;
const SMART_PREDICTOR_TABLE_SIZE: usize = 2048;
const MAX_HISTORY_LENGTH: usize = 256;

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
        alt_table: usize,

        provider_predictor: Option<SmartPredictionEntry>,
        alt_predictor: Option<SmartPredictionEntry>,
    },
    None,
}

#[derive(Clone, Copy)]
struct PredictionResult {
    taken: bool,
    meta: PredictionResultMeta,
}

struct CSR {
    val: u32,
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
        let mut new_val = (self.val << 1) | (self.val >> (self.out_bits - 1));
        new_val &= (1 << self.out_bits) - 1;
        new_val ^= new_bit as u32;

        let old_pos = self.hist_len % self.out_bits;
        new_val ^= (old_bit as u32) << old_pos;

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
        let mut alt_predictor: Option<SmartPredictionEntry> = None;
        let mut alt_table: usize = 0;

        for (i, predictor) in self.smart_predictors.iter().enumerate().rev() {
            let table_index = predictor.get_table_index(trace_line.pc as usize);
            let table_entry = predictor.prediction_table[table_index];

            if table_entry.tag == predictor.get_tag(trace_line.pc as usize) {
                match (provider_predictor, alt_predictor) {
                    (None, _) => {
                        provider_predictor = Some(table_entry);
                        provider_table = i;
                    }
                    (Some(_), None) => {
                        alt_predictor = Some(table_entry);
                        alt_table = i;
                        break;
                    }
                    _ => {}
                }
            }
        }
        return PredictionResultMeta::Tagged {
            provider_table,
            alt_table,
            provider_predictor,
            alt_predictor,
        };
    }

    fn predict(&self, trace_line: &TraceLine) -> PredictionResult {
        let predict_smart = self.predicict_smart(trace_line);
        let predict_base = self.predict_base(trace_line);

        let mut final_prediction: PredictionResult = match predict_base {
            PredictionResultMeta::Base { prediction, .. } => PredictionResult {
                taken: prediction,
                meta: predict_base,
            },
            _ => unreachable!(),
        };

        match predict_smart {
            PredictionResultMeta::None => {}
            PredictionResultMeta::Base {..}=> {
                unreachable!()
            }
            PredictionResultMeta::Tagged {
                provider_predictor,..
            } => {
                if !provider_predictor.is_none() {
                    final_prediction = PredictionResult {
                        taken: provider_predictor.unwrap().decide(),
                        meta: predict_smart,
                    };
                }
            }
        }
        return final_prediction;
    }
    fn update(&mut self, prediction: PredictionResult, actual_result: bool) {
        match prediction.meta {
            PredictionResultMeta::Base {
                prediction,
                t0_index,
            } => {
                let counter = &mut self.base_predictor[t0_index];

                if actual_result {
                    *counter = Utils::bounded_increment(*counter, 3);
                } else {
                    if *counter > 0 {
                        *counter = Utils::bounded_decrement(*counter);
                    }
                }
            }
            PredictionResultMeta::Tagged {
                provider_table,
                alt_table,
                provider_predictor,
                alt_predictor,
            } => {
                todo!()
            }
            _ => {}
        }
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
        tage.update(prediction, trace_line.taken);
        stats.add_result(prediction.taken == trace_line.taken);
    }
    Ok(stats)
}

fn main() -> io::Result<()> {
    let mut tage = Tage::new(SMART_PREDICTOR_AMOUNT);

    for i in 1..=10 {
        let path_str = format!("traces/trace_{i:02}");
        let path = Path::new(&path_str);

        let stats = run_trace(path, &mut tage).expect("bad trace stats");
        println!("{}", stats.get_result());
    }
    Ok(())
}
