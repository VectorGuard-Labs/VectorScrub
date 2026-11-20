"""The removal browser is openly automated. These tests keep it that way.

The product's rule (README, "Removal automation is honest about what blocks it"): never defeat a
site's bot protection. No disguised browser, no CAPTCHA solving, no proxy rotation. A site that
refuses the browser is reported as refusing and the customer is pointed to the manual guide.
This reads the source, so a "stealth" feature cannot come back under another name unnoticed.
"""
import re
from pathlib import Path

AGENTS = Path(__file__).resolve().parent.parent

# Things whose only purpose is to make an automated browser pass as a person's.
DISGUISES = {
    "hides the webdriver flag": re.compile(r"navigator\s*,\s*['\"]webdriver['\"]|navigator\.webdriver\s*=|webdriver\s*['\"]?\s*:\s*undefined"),
    "turns off Chromium's automation marker": re.compile(r"AutomationControlled"),
    "stealth plugin": re.compile(r"playwright[_-]extra|puppeteer-extra|playwright[_-]stealth|undetected", re.I),
    "fakes browser plugins or languages": re.compile(r"defineProperty\(\s*navigator\s*,\s*['\"](plugins|languages|platform|hardwareConcurrency)['\"]"),
    "rotates user agents": re.compile(r"\bUSER_AGENTS\b|get_random_user_agent|get_user_agent_for_broker|fingerprint[_ ]rotat", re.I),
    "imitates a person's input": re.compile(r"human_like|humanlike|bezier|mouse[_ ]movement", re.I),
    "CAPTCHA solving service": re.compile(r"2captcha|anticaptcha|capsolver|deathbycaptcha|solve_captcha", re.I),
    "proxy rotation": re.compile(r"proxy[_ ]rotat|rotating[_ ]prox|brightdata|smartproxy|luminati", re.I),
}


def _source_files():
    for path in AGENTS.rglob("*.py"):
        parts = set(path.relative_to(AGENTS).parts)
        if parts & {"tests", "__pycache__", ".venv", "venv"}:
            continue
        yield path


def test_no_module_exists_to_disguise_the_browser():
    names = [p.name for p in _source_files() if re.search(r"stealth|evasion|undetect|fingerprint", p.name, re.I)]
    assert names == [], names


def test_nothing_in_the_agents_disguises_the_browser_or_gets_around_a_block():
    found = []
    for path in _source_files():
        text = path.read_text(encoding="utf-8", errors="replace")
        for why, pattern in DISGUISES.items():
            if pattern.search(text):
                found.append(f"{path.relative_to(AGENTS)}: {why}")
    assert found == [], "\n".join(found)


def test_the_executor_launches_a_plain_context():
    src = (AGENTS / "removal" / "executor.py").read_text(encoding="utf-8")
    assert "browser.new_context()" in src
    assert "user_agent" not in src.split("async with async_playwright()")[1].split("response = await page.goto")[0]
