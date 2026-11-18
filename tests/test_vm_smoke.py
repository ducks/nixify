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

    def test_expected_commands_require_actual_console_markers(self):
        console = b'for cmd in Hyprland; do echo NIXIFY_COMMAND_$cmd=present; done\r\nNIXIFY_COMMAND_Hyprland=present\r\n'
        self.assertEqual(vm_smoke.missing_commands(console, ["Hyprland"]), [])
        self.assertEqual(vm_smoke.missing_commands(console, ["nvim"]), ["nvim"])

    def test_command_names_cannot_contain_shell_syntax_or_paths(self):
        for value in ["sh;touch", "../bin/sh", "$(id)", "-n", ""]:
            with self.subTest(value=value), self.assertRaises(Exception):
                vm_smoke.command_name(value)
        self.assertEqual(vm_smoke.command_name("Hyprland"), "Hyprland")


if __name__ == "__main__":
    unittest.main()
