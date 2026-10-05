# Run from assets/screenshots with REVIA_BIN_DIR pointing at a built revia.
Require revia

Set Shell bash
Set FontFamily "PlemolJP Console NF"
Set FontSize 16
Set Width 920
Set Height 720
Set Theme "Catppuccin Mocha"
Set Padding 16
Set TypingSpeed 40ms
Set CursorBlink false

Hide
Type "source ../capture/env.sh"
Enter
Type "revia"
Enter
Wait+Screen /src\/lib\.rs/
Type "."
Sleep 1s
Show
Sleep 500ms
Screenshot "review-stack.png"
