use std::fs::File;
use std::io::{self, BufRead, BufReader};
use std::path::Path;

const BASE_PREDICTOR_SIZE: usize = 4096;
const SMART_PREDICTOR_AMOUNT: usize = 6;
const SMART_PREDICTOR_FIRST_SIZE: usize = 5;
const SMART_PREDICTOR_TABLE_SIZE: usize = 2048;

#[derive(Debug)]
struct TraceLine {
    pc: u64,
    taken: bool,
}

impl TraceLine {
    fn parse(line: &str) -> Self {
        let mut parts = line.split_whitespace();

        let pc_str = parts.next().expect("pc not a number");
        let pc_str = pc_str.trim_start_matches("0x");

        let taken_str = parts.next().expect("pc not a number");
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
}

struct SmartPredictor {
    perspicacity: u8,
    prediction_table: Vec<SmartPredictionEntry>,
}

impl SmartPredictor {
    fn new(perspicacity: u8) -> Self {
        Self {
            prediction_table: vec![SmartPredictionEntry::new(); SMART_PREDICTOR_TABLE_SIZE],
            perspicacity,
        }
    }
}

struct Tage {
    base_predictor: Vec<u16>,
    smart_predictors: Vec<SmartPredictor>,
}

#[derive(Clone, Copy)]
struct PredictionResult {
    taken: bool,
    t0_index: usize,
}

struct Utils {}
impl Utils {
    const fn get_perspicacity(i: usize) -> usize {
        return SMART_PREDICTOR_FIRST_SIZE + i * i;
    }

    const fn get_t0_index(pc: u64) -> usize {
        (pc & 0xFFF) as usize // get last 12 bit
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
        }
    }
    fn predict_base(&self, index: usize) -> bool {
        let prediction = self.base_predictor[index];
        return prediction > 0b01;
    }

    fn predict(&self, trace_line: &TraceLine) -> PredictionResult {
        let index = Utils::get_t0_index(trace_line.pc);

        PredictionResult {
            t0_index: index,
            taken: self.predict_base(index),
        }
    }
    fn update(&mut self, meta: PredictionResult, actual_result: bool) {
        let counter = &mut self.base_predictor[meta.t0_index];

        if actual_result {
            if *counter < 3 {
                *counter += 1;
            }
        } else {
            if *counter > 0 {
                *counter -= 1;
            }
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
