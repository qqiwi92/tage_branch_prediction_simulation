use std::fs::File;
use std::io::{self, BufRead, BufReader};
use std::path::Path;

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

fn main() -> io::Result<()> {
    let path = Path::new("traces/trace_01");
    let file = File::open(path)?;
    let reader = BufReader::new(file);

    for line in reader.lines() {
        let line = line?;
        let trace_line = TraceLine::parse(&line);
        println!("{:?}", trace_line);
    }
    Ok(())
}
