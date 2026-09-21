Output docs/assets/xpdelve.gif

Require target/release/xpdelve
Require crossplane

Set Shell "zsh"
Set FontSize 18
Set Width 1400
Set Height 820
Set Framerate 18
Set TypingSpeed 45ms
Set CursorBlink false
Set Theme {"name":"Catppuccin Mocha","black":"#45475a","red":"#f38ba8","green":"#a6e3a1","yellow":"#f9e2af","blue":"#89b4fa","magenta":"#cba6f7","cyan":"#94e2d5","white":"#bac2de","brightBlack":"#585b70","brightRed":"#f38ba8","brightGreen":"#a6e3a1","brightYellow":"#f9e2af","brightBlue":"#89b4fa","brightMagenta":"#cba6f7","brightCyan":"#94e2d5","brightWhite":"#a6adc8","background":"#1e1e2e","foreground":"#cdd6f4","selection":"#585b70","cursor":"#f5e0dc"}

Hide
Type "target/release/xpdelve --kubeconfig target/demo/kubeconfig --namespace xpdelve-demo --watch-interval 1s DemoPlatform.demo.xpdelve.io/xpdelve-demo"
Enter
Wait+Screen /OBJECT/
Show
Sleep 2s

# Show that the complete nested trace can be collapsed and expanded.
Type "["
Sleep 1s
Type "]"
Sleep 2s

# Inspect the nested application XR as live YAML.
Down 4
Sleep 1s
Type "y"
Sleep 3s
Type "q"
Sleep 1s

# Demonstrate tree filtering while retaining the ancestor path.
Type "/"
Type "network"
Enter
Sleep 3s
Escape
Sleep 1s

# Delete the root using the safe foreground default.
Type "g"
Ctrl+D
Sleep 3s
Enter
Sleep 4s

# Find the leaf that deliberately blocks the cascading deletion.
Type "/"
Type "network-settings"
Enter
Sleep 2s
Down 2
Ctrl+X
Sleep 3s
Enter

# The cascade completes and xpdelve clears the deleted root trace.
Sleep 8s
