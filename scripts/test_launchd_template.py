import plistlib
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
TEMPLATE = ROOT / "scripts/launchd/com.kent-tokyo.sekirei-floodgate.plist.template"


def test_launchd_template_is_valid_and_secret_free():
    document = plistlib.loads(TEMPLATE.read_bytes())
    assert document["Label"] == "com.kent-tokyo.sekirei-floodgate"
    arguments = document["ProgramArguments"]
    assert "--" in arguments
    assert all(not token.lower().startswith(("password=", "token=")) for token in arguments)
    assert "__CSA_BINARY__" in arguments
    assert "__FLOODGATE_USER__" in arguments
    assert "__EVAL_MODE__" in arguments
    assert "--client-status-file" in arguments
    assert "--max-log-bytes" in arguments
    assert document["KeepAlive"] == {"SuccessfulExit": False}
    assert document["ThrottleInterval"] == 30


if __name__ == "__main__":
    test_launchd_template_is_valid_and_secret_free()
    print("PASS")
