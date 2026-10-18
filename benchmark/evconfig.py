"""eval 的统一配置加载。

配置在 eval/config.toml；本模块只做读取与路径解析，不写回。
"""

from __future__ import annotations

import tomllib
from dataclasses import dataclass
from pathlib import Path
from typing import Any

EVAL_ROOT = Path(__file__).resolve().parent
CONFIG_PATH = EVAL_ROOT / "config.toml"


class ConfigError(RuntimeError):
    pass


@dataclass
class Config:
    raw: dict[str, Any]
    path: Path

    # ---- 路径 ----------------------------------------------------------
    @property
    def testset(self) -> Path:
        return Path(self.raw["paths"]["testset"])

    @property
    def judge(self) -> Path:
        return Path(self.raw["paths"]["judge"])

    @property
    def reports(self) -> Path:
        return Path(self.raw["paths"]["reports"])

    @property
    def specs(self) -> Path:
        return EVAL_ROOT / "specs"

    @property
    def fixtures(self) -> Path:
        return EVAL_ROOT / "fixtures"

    def section(self, name: str) -> dict[str, Any]:
        return self.raw.get(name, {})

    def get(self, dotted: str, default: Any = None) -> Any:
        cur: Any = self.raw
        for part in dotted.split("."):
            if not isinstance(cur, dict) or part not in cur:
                return default
            cur = cur[part]
        return cur


def load_config(path: Path | None = None) -> Config:
    p = Path(path) if path else CONFIG_PATH
    if not p.exists():
        raise ConfigError(f"配置文件不存在：{p}")
    with p.open("rb") as fh:
        raw = tomllib.load(fh)
    cfg = Config(raw=raw, path=p)
    # 早失败：路径类配置拼错的话，后面每一步都会以一个更难懂的方式报错
    missing = [k for k in ("paths.testset", "paths.judge") if cfg.get(k) is None]
    if missing:
        raise ConfigError(f"配置缺少：{', '.join(missing)}")
    return cfg
