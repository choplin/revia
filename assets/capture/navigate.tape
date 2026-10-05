# Run from assets/screenshots with REVIA_BIN_DIR pointing at a built revia.
Require revia
Output "navigate.gif"

Set Shell bash
Set FontFamily "PlemolJP Console NF"
Set FontSize 16
Set Width 1200
Set Height 720
Set Theme "Catppuccin Mocha"
Set Padding 16
Set TypingSpeed 120ms
Set CursorBlink false
Set Framerate 20

Hide
Type "source ../capture/env.sh"
Enter
Type "revia"
Enter
Wait+Screen /docs\/guide\.md/
Sleep 500ms
Show
Sleep 1s
Type "."
Sleep 1s
Type "."
Sleep 1s
Type "]"
Sleep 1.2s
Type "/"
Sleep 300ms
Type "BASE_URL"
Sleep 400ms
Enter
Sleep 1.2s
Type "n"
Sleep 1.5s
