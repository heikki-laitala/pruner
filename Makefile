.PHONY: install test lint format check clean index

install:
	uv sync

test:
	uv run pytest -v

test-quick:
	uv run pytest -x -q

coverage:
	uv run pytest --cov=pruner --cov-report=term-missing

lint:
	uv run ruff check src/ tests/

format:
	uv run ruff format src/ tests/

check: lint test

clean:
	rm -rf .pruner/ .pytest_cache/ .ruff_cache/
	find . -type d -name __pycache__ -exec rm -rf {} +

index:
	uv run pruner index . -v
