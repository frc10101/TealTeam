#!/usr/bin/env python3
"""Best-effort LCD writer for a 16x2 I2C display using RPLCD."""

import argparse
import sys


def _fit(text: str, width: int = 16) -> str:
    text = (text or "").strip()
    if len(text) > width:
        return text[:width]
    return text.ljust(width)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--line1", default="TealTeam")
    parser.add_argument("--line2", default="")
    parser.add_argument("--address", default="0x27", help="I2C address, usually 0x27 or 0x3f")
    parser.add_argument("--port", type=int, default=1, help="I2C bus (usually 1 on Raspberry Pi)")
    args = parser.parse_args()

    try:
        from RPLCD.i2c import CharLCD
    except Exception:
        # Keep boot startup flow non-fatal if LCD deps are not installed.
        return 0

    try:
        address = int(args.address, 16) if isinstance(args.address, str) else int(args.address)
        lcd = CharLCD(i2c_expander="PCF8574", address=address, port=args.port, cols=16, rows=2)
        lcd.clear()
        lcd.cursor_pos = (0, 0)
        lcd.write_string(_fit(args.line1))
        lcd.cursor_pos = (1, 0)
        lcd.write_string(_fit(args.line2))
        return 0
    except Exception:
        # Non-fatal by design to avoid blocking startup.
        return 0


if __name__ == "__main__":
    sys.exit(main())
