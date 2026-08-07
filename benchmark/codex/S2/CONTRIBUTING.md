# Contributing

Thanks for wanting to contribute to Todo Manager!

## Getting started

1. Fork the repository and clone it locally.
2. Install the development dependencies:

   ```bash
   pip install -e ".[dev]"
   ```

3. Create a feature branch: `git checkout -b feat/my-change`.

## Tests

Run the full suite before submitting a pull request:

```bash
pytest -q
```

## Pull request checklist

- [ ] Describe the change and why it is needed.
- [ ] Add or update tests for changed behavior.
- [ ] Run `pytest -q` and make sure all tests pass.
- [ ] Keep the diff focused on one concern.
