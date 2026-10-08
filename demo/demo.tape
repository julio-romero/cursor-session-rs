# Records demo/demo.gif for the README, from the invented sessions that
# make-fixture.py writes. From the repository root:
#
#   cargo build --release
#   python3 demo/make-fixture.py
#   vhs demo/demo.tape

Output demo/demo.gif

Set Shell "bash"
Set FontSize 16
Set Width 1360
Set Height 560
Set Padding 20
Set Theme "Catppuccin Mocha"
Set WindowBar Colorful
Set TypingSpeed 40ms
Env BASH_SILENCE_DEPRECATION_WARNING "1"

Hide
Type 'export HOME="$PWD/demo/home" PATH="$PWD/target/release:$PATH"; cd "$(mktemp -d)"; clear'
Enter
Sleep 500ms
Show

Type "cursor-session list"
Sleep 500ms
Enter
Sleep 4s
Ctrl+L

Type "cursor-session show a71d"
Sleep 500ms
Enter
Sleep 5s
Ctrl+L

Type `cursor-session list --json | jq -r '.[] | select(.source == "ide") | .title'`
Sleep 500ms
Enter
Sleep 3.5s
Ctrl+L

Type "cursor-session export --session-id 3f9c && head -n 15 exports/*.md"
Sleep 500ms
Enter
Sleep 5s
