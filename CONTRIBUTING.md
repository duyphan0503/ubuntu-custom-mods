# Contributing to Ubuntu Custom Mods

Thank you for considering contributing to `ubuntu-custom-mods`! Contributions from developers, designers, and Linux enthusiasts are warmly welcomed.

---

## 🛠️ Development Setup

### Prerequisites
- **Ubuntu Desktop** (24.04 LTS or 26.04 LTS recommended) with GNOME Shell 46+
- **Rust toolchain** (`rustc`, `cargo` latest stable)
- **C/C++ build tools**: `build-essential`, `pkg-config`
- **Libraries**:
  - `libglib2.0-dev`
  - `libgtk-4-dev`
  - `libnautilus-extension-dev` (or 4.0)
  - `wl-clipboard` (for Wayland clipboard integration)

```bash
sudo apt update
sudo apt install -y build-essential pkg-config libglib2.0-dev libgtk-4-dev wl-clipboard
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

---

## 🔀 Branching Strategy

1. Fork the repository and create your branch from `main`:
   ```bash
   git checkout -b feat/your-feature-name
   # or
   git checkout -b fix/your-bugfix-name
   ```
2. Keep branches focused on a single topic or bug fix.

---

## 💬 Git Commit Conventions

We strictly follow the **[Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/)** specification:

Format: `<type>(<scope>): <short description>`

### Allowed Types:
- `feat`: A new feature or enhancement.
- `fix`: A bug fix.
- `docs`: Documentation updates.
- `style`: Formatting, missing semicolons, etc. (no production code change).
- `refactor`: Refactoring code without altering features or bug fixes.
- `perf`: Code changes that improve performance.
- `test`: Adding or correcting tests.
- `ci`: Changes to CI/CD workflows and automation scripts.
- `chore`: Maintenance tasks, dependency updates.

### Examples:
- `feat(clippaste): add image thumbnail rendering in GNOME topbar`
- `fix(nautilus): resolve memory leak in async worker thread`
- `perf(daemon): reduce memory footprint in shm cache`

---

## 🧪 Code Quality & Testing

Before submitting your PR, ensure that code complies with standard formatters and linters:

```bash
# Check formatting
cargo fmt --all -- --check

# Run linter
cargo clippy --all-targets --all-features -- -D warnings

# Run unit and integration tests
cargo test --all
```

---

## 📬 Submitting a Pull Request (PR)

1. Ensure all status checks and GitHub Actions CI pipelines pass.
2. Provide a clear, descriptive title and fill out the [PR Template](.github/PULL_REQUEST_TEMPLATE.md).
3. If this PR closes an existing issue, link it using `Closes #123`.
4. Be responsive to code review feedback.
