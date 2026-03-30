
TellurCom is live at 969K. 


TellurCom — a fully functional Midnight Commander-style file manager in Rust.

What's included:

    Dual-pane layout — left panel starts in your home directory, right panel starts at /
    Dark blue theme — classic MC-style colors with cyan borders, cyan highlights, green dates, yellow sizes
    File listing — shows name, size (auto-formatted: B/K/M/G), and modification date
    Color coding — directories in cyan, executables in green, symlinks in magenta
    Scrollbar — block character scrollbar on each panel
    Status bar — shows item count, Unix permissions, size, and filename
    F-key bar at the bottom (F1–F10)

Navigation:
Key	Action
↑↓ / j k	Move cursor
Enter / →	Open directory
Backspace / ←	Go to parent
Tab	Switch panels
PgUp/PgDn, Home/End	Jump through list

File operations:
Key	Action
F3	View file contents
F5	Copy (destination pre-filled from other panel)
F6	Move/rename to other panel
F7	Create directory
F8	Delete with confirmation dialog
F9	Rename file or directory
F10 / q	Quit

The binary runs at ~1ms startup time and uses ~1MB RAM. 
