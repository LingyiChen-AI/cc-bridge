use std::{collections::VecDeque, sync::Mutex};

pub const LOG_CAP: usize = 500;

#[derive(Default)]
pub struct LogBuffer(Mutex<VecDeque<String>>);

impl LogBuffer {
    pub fn push(&self, line: String) {
        let mut q = self.0.lock().unwrap();
        if q.len() == LOG_CAP {
            q.pop_front();
        }
        q.push_back(line);
    }
    pub fn len(&self) -> usize {
        self.0.lock().unwrap().len()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn tail(&self, n: usize) -> Vec<String> {
        let q = self.0.lock().unwrap();
        q.iter().skip(q.len().saturating_sub(n)).cloned().collect()
    }
    pub fn all(&self) -> Vec<String> {
        self.0.lock().unwrap().iter().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn log_buffer_caps_500() {
        let b = LogBuffer::default();
        for i in 1..=600 {
            b.push(format!("line {i}"));
        }
        assert_eq!(b.len(), 500);
        assert_eq!(b.all()[0], "line 101");
        assert_eq!(b.tail(2), ["line 599", "line 600"]);
    }
}
