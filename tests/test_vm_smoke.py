import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("vm_smoke", Path(__file__).with_name("run-vm-smoke.py"))
vm_smoke = importlib.util.module_from_spec(spec)
spec.loader.exec_module(vm_smoke)


class ConsoleDetectionTest(unittest.TestCase):
    def test_colored_root_prompt_with_title(self):
        prompt = b"\x1b[1;31m[\x1b]0;root@nixify-test: ~\x07root@nixify-test:~]#\x1b[0m "
        self.assertTrue(vm_smoke.guest_prompt(prompt))

    def test_prompt_must_be_from_test_guest(self):
        self.assertFalse(vm_smoke.guest_prompt(b"[root@other:~]# "))

    def test_echoed_command_does_not_count_as_completion(self):
        self.assertFalse(vm_smoke.test_done(b"printf 'NIXIFY_VM_TEST_DONE\\n'; poweroff\r\n"))
        self.assertTrue(vm_smoke.test_done(b"\r\nNIXIFY_VM_TEST_DONE\r\n"))


if __name__ == "__main__":
    unittest.main()
