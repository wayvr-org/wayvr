use std::{
    collections::VecDeque,
    io::Read,
    os::{fd::AsFd, unix::net::UnixStream},
};

use crate::subsystem::monado_metrics::proto;

pub struct MonadoMetricsFd {
    stream_reader: UnixStream,
    stream_writer: UnixStream,

    records: VecDeque<proto::Record>,
}

const RECORD_QUEUE_SIZE: usize = 500;

impl MonadoMetricsFd {
    pub fn new(monado: &mut libmonado::Monado) -> anyhow::Result<Self> {
        let (stream_reader, stream_writer) = std::os::unix::net::UnixStream::pair()?;
        stream_writer.set_nonblocking(true)?;
        stream_reader.set_nonblocking(true)?;

        monado.push_metrics_fd(stream_writer.as_fd(), true)?;

        Ok(Self {
            stream_reader,
            stream_writer,
            records: VecDeque::new(),
        })
    }

    fn parse_message(&mut self, record: proto::Record) {
        self.records.push_back(record);
        if self.records.len() >= RECORD_QUEUE_SIZE {
            self.records.pop_front();
        }
    }

    pub fn dump_records(&mut self) -> Vec<proto::Record> {
        let records = std::mem::take(&mut self.records);
        records.into_iter().collect()
    }

    pub fn is_full(&self) -> bool {
        self.records.len() >= RECORD_QUEUE_SIZE - 1
    }

    // called every frame
    pub fn update(&mut self) {
        let mut buf: [u8; 1024] = [0; 1024];

        while let Ok(byte_count) = self.stream_reader.read(&mut buf) {
            if byte_count == 0 {
                debug_assert!(false);
                break;
            }

            let res: Result<proto::Record, _> = prost::Message::decode_length_delimited(&buf[..]);
            match res {
                Ok(record) => {
                    self.parse_message(record);
                }
                Err(e) => {
                    log::error!("decode error: {e}");
                }
            }
        }
    }
}
