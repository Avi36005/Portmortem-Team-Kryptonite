.PHONY: help build test verify fuzz bench oracle hunt unit clean setup all demo

VENV        := .venv
VENV_BASE   := .venv-baseline
PY          := $(VENV)/bin/python
PY_BASE     := $(VENV_BASE)/bin/python
CARGO       := cargo
BIN         := target/release/croniter

# sha256sum on Linux, shasum -a 256 on macOS without coreutils. Same format.
SHASUM := $(shell command -v sha256sum >/dev/null 2>&1 && echo "sha256sum" || echo "shasum -a 256")

help:
	@echo "make build    Build the shipped artifact (target/release/croniter)"
	@echo "make test     Build the PyO3 bridge and run the ORIGINAL test suite"
	@echo "make unit     Run the Rust unit + CLI integration tests"
	@echo "make verify   Check tests/original/ fingerprints AND that core/ has no unsafe"
	@echo "make fuzz     120s differential fuzz: Python original vs Rust port"
	@echo "make bench    Benchmark both implementations (p50/p95/p99 + RSS)"
	@echo "make hunt     Hunt for upstream croniter bugs (invariant harnesses)"
	@echo "make setup    Create both virtualenvs from scratch"
	@echo "make demo     Run every claim live (for the demo video)"
	@echo "make all      verify + build + unit + test"

build:
	$(CARGO) build --release -p croniter-core
	@echo "built $(BIN)"

unit:
	$(CARGO) test -p croniter-core

test:
	VIRTUAL_ENV=$(CURDIR)/$(VENV) $(VENV)/bin/maturin develop -m pybridge/Cargo.toml
	$(PY) -m pytest tests/original/ -q

# Two things a judge should be able to confirm in one command.
verify:
	@echo "== tests/original/ fingerprints =="
	@$(SHASUM) -c .test-hashes.sha256
	@echo
	@echo "== unsafe blocks in core/ (must be 0) =="
	@grep -rn "unsafe" core/src/ | grep -v "forbid(unsafe_code)" | wc -l | tr -d ' '
	@echo
	@echo "== pyo3 in the shipped crate's dependency tree (must be 0) =="
	@$(CARGO) tree -p croniter-core --edges normal 2>/dev/null | grep -ci pyo3 || true

fuzz:
	$(PY) fuzz/differential.py --seconds 120 | tee fuzz/log.txt

bench: build
	$(PY_BASE) bench/run_bench.py 25

# The upstream bug hunt. Three harnesses; triage filters croniter's documented
# skip-forward behaviour out of the field-checker's raw output.
hunt:
	$(PY_BASE) fuzz/invariants.py  --seconds 120
	$(PY_BASE) fuzz/invariants2.py --seconds 120
	$(PY_BASE) fuzz/triage.py

oracle:
	$(PY_BASE) fuzz/oracle.py

setup:
	python3 -m venv $(VENV_BASE)
	$(VENV_BASE)/bin/pip install -q --upgrade pip
	$(VENV_BASE)/bin/pip install -q "croniter @ git+https://github.com/pallets-eco/croniter@$$(cat .kickoff-commit)" \
		python-dateutil pytz pytest
	python3 -m venv $(VENV)
	$(VENV)/bin/pip install -q --upgrade pip
	$(VENV)/bin/pip install -q maturin pytest python-dateutil pytz

demo: build
	bash scripts/demo.sh

all: verify build unit test

clean:
	$(CARGO) clean
	rm -rf $(VENV) $(VENV_BASE)
