#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Operation(u16);

impl Operation {
    pub fn new(code: u16) -> Self {
        Self(code)
    }

    pub fn code(&self) -> u16 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OperationSpecification {
    operation: Operation,
    name: &'static str,
    argument_widths: &'static [usize],
    payload_widths: &'static [usize],
}

impl OperationSpecification {
    pub fn operation(&self) -> Operation {
        self.operation
    }

    pub fn name(&self) -> &'static str {
        self.name
    }

    pub fn argument_widths(&self) -> &'static [usize] {
        self.argument_widths
    }

    pub fn payload_widths(&self) -> &'static [usize] {
        self.payload_widths
    }

    pub fn result_width(&self) -> usize {
        1 + self.payload_widths.iter().sum::<usize>()
    }
}

pub const OPERATIONS: &[OperationSpecification] = &[
    OperationSpecification {
        operation: Operation(0x0000),
        name: "process.exit",
        argument_widths: &[8],
        payload_widths: &[],
    },
    OperationSpecification {
        operation: Operation(0x0100),
        name: "memory.acquire",
        argument_widths: &[64],
        payload_widths: &[64],
    },
    OperationSpecification {
        operation: Operation(0x0101),
        name: "memory.release",
        argument_widths: &[64],
        payload_widths: &[],
    },
    OperationSpecification {
        operation: Operation(0x0102),
        name: "memory.read",
        argument_widths: &[64, 64],
        payload_widths: &[8],
    },
    OperationSpecification {
        operation: Operation(0x0103),
        name: "memory.write",
        argument_widths: &[64, 64, 8],
        payload_widths: &[],
    },
    OperationSpecification {
        operation: Operation(0x0200),
        name: "stream.open",
        argument_widths: &[64, 64, 64, 8],
        payload_widths: &[64],
    },
    OperationSpecification {
        operation: Operation(0x0201),
        name: "stream.read",
        argument_widths: &[64],
        payload_widths: &[8],
    },
    OperationSpecification {
        operation: Operation(0x0202),
        name: "stream.write",
        argument_widths: &[64, 8],
        payload_widths: &[],
    },
    OperationSpecification {
        operation: Operation(0x0203),
        name: "stream.flush",
        argument_widths: &[64],
        payload_widths: &[],
    },
    OperationSpecification {
        operation: Operation(0x0204),
        name: "stream.close",
        argument_widths: &[64],
        payload_widths: &[],
    },
    OperationSpecification {
        operation: Operation(0x0205),
        name: "stream.read.block",
        argument_widths: &[64, 64, 64, 64],
        payload_widths: &[64],
    },
    OperationSpecification {
        operation: Operation(0x0206),
        name: "stream.write.block",
        argument_widths: &[64, 64, 64, 64],
        payload_widths: &[],
    },
    OperationSpecification {
        operation: Operation(0x0300),
        name: "clock.monotonic",
        argument_widths: &[],
        payload_widths: &[64],
    },
    OperationSpecification {
        operation: Operation(0x0301),
        name: "clock.realtime",
        argument_widths: &[],
        payload_widths: &[64],
    },
    OperationSpecification {
        operation: Operation(0x0400),
        name: "random.byte",
        argument_widths: &[],
        payload_widths: &[8],
    },
];

pub fn lookup(code: u16) -> Option<&'static OperationSpecification> {
    OPERATIONS
        .iter()
        .find(|specification| specification.operation.code() == code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup_finds_memory_acquire() {
        let specification = lookup(0x0100).expect("memory.acquire is in the table");
        assert_eq!(specification.name(), "memory.acquire");
        assert_eq!(specification.argument_widths(), &[64]);
        assert_eq!(specification.payload_widths(), &[64]);
        assert_eq!(specification.result_width(), 65);
    }

    #[test]
    fn lookup_finds_process_exit() {
        let specification = lookup(0x0000).expect("process.exit is in the table");
        assert_eq!(specification.name(), "process.exit");
        assert_eq!(specification.result_width(), 1);
    }

    #[test]
    fn lookup_rejects_an_unknown_code() {
        assert!(lookup(0x7fff).is_none());
    }

    #[test]
    fn every_operation_has_a_unique_code() {
        for (index, specification) in OPERATIONS.iter().enumerate() {
            for other in &OPERATIONS[index + 1..] {
                assert_ne!(specification.operation().code(), other.operation().code());
            }
        }
    }

    #[test]
    fn every_operation_declares_its_widths() {
        for specification in OPERATIONS {
            assert!(!specification.name().is_empty());
            assert!(specification.argument_widths().len() <= 4);
            assert!(specification.result_width() >= 1);
            for width in specification
                .argument_widths()
                .iter()
                .chain(specification.payload_widths())
            {
                assert!(*width == 8 || *width == 64);
            }
        }
    }

    #[test]
    fn the_table_covers_every_group() {
        let mut groups: Vec<u16> = OPERATIONS
            .iter()
            .map(|specification| specification.operation().code() >> 8)
            .collect();
        groups.sort_unstable();
        groups.dedup();
        assert_eq!(groups, vec![0x00, 0x01, 0x02, 0x03, 0x04]);
    }

    #[test]
    fn the_reserved_group_is_unassigned() {
        for specification in OPERATIONS {
            assert!(specification.operation().code() < 0x8000);
        }
    }

    #[test]
    fn the_result_begins_with_the_status_bit() {
        for specification in OPERATIONS {
            assert_eq!(
                specification.result_width(),
                1 + specification.payload_widths().iter().sum::<usize>()
            );
        }
    }
}
