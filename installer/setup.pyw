"""DOOM RING setup wizard (tkinter). The work itself: engine.py."""
import os
import subprocess
import sys
import tkinter as tk
from pathlib import Path
from tkinter import filedialog, messagebox, ttk

sys.path.insert(0, str(Path(__file__).resolve().parent))
import engine  # noqa: E402

BG = "#16130f"
PANEL = "#221d18"
FG = "#e8e2d8"
DIM = "#a59c8f"
RED = "#c8261b"
GREEN = "#7bc043"
WARN = "#e0a33a"
TITLE_FONT = ("Segoe UI", 20, "bold")
BODY_FONT = ("Segoe UI", 10)
SMALL_FONT = ("Segoe UI", 9)


class Wizard(tk.Tk):
    def __init__(self):
        super().__init__()
        self.title(f"DOOM RING Version {engine.setup_version()['version']} Setup")
        self.geometry("760x560")
        self.minsize(700, 520)
        self.configure(bg=BG)
        icon = engine.PACKAGE / "doomring.ico"
        if icon.exists():
            try:
                self.iconbitmap(str(icon))
            except tk.TclError:
                pass
        style = ttk.Style(self)
        style.theme_use("clam")
        style.configure("TFrame", background=BG)
        style.configure("Panel.TFrame", background=PANEL)
        style.configure("TLabel", background=BG, foreground=FG, font=BODY_FONT)
        style.configure("Panel.TLabel", background=PANEL, foreground=FG, font=BODY_FONT)
        style.configure("Title.TLabel", background=BG, foreground=FG, font=TITLE_FONT)
        style.configure("Dim.TLabel", background=BG, foreground=DIM, font=SMALL_FONT)
        style.configure("TButton", font=BODY_FONT, padding=(14, 6), background="#3a332b", foreground=FG,
                        bordercolor="#5a5046")
        style.map("TButton", background=[("active", "#4a4036"), ("disabled", "#2a2520")],
                  foreground=[("disabled", "#6d655b")])
        style.configure("Accent.TButton", background=RED, foreground="white")
        style.map("Accent.TButton", background=[("active", "#e0372b"), ("disabled", "#5a2a26")])
        style.configure("TCheckbutton", background=BG, foreground=FG, font=BODY_FONT)
        style.map("TCheckbutton", background=[("active", BG)])
        style.configure("red.Horizontal.TProgressbar", troughcolor=PANEL, background=RED, bordercolor=PANEL,
                        lightcolor=RED, darkcolor=RED)

        self.er_path = engine.find_app(engine.ELDEN_RING_APP)
        self.doom_path = engine.find_app(engine.DOOM_ETERNAL_APP)
        self.target = tk.StringVar(value=str(self.default_target()))
        self.shortcut = tk.BooleanVar(value=True)
        self.installer = None
        self.version = engine.setup_version()["version"]
        self.updating = False
        self.space = self.found = None
        self.target.trace_add("write", lambda *_: self.update_space())

        self.body = ttk.Frame(self, padding=(28, 22, 28, 8))
        self.body.pack(fill="both", expand=True)
        bar = ttk.Frame(self, padding=(28, 8, 28, 18))
        bar.pack(fill="x", side="bottom")
        self.back_btn = ttk.Button(bar, text="< Back", command=self.back)
        self.next_btn = ttk.Button(bar, text="Next >", style="Accent.TButton", command=self.next)
        self.cancel_btn = ttk.Button(bar, text="Cancel", command=self.cancel)
        self.cancel_btn.pack(side="right")
        self.next_btn.pack(side="right", padx=(0, 10))
        self.back_btn.pack(side="right", padx=(0, 10))
        self.pages = [self.page_welcome, self.page_games, self.page_location, self.page_install, self.page_done]
        self.page = 0
        self.protocol("WM_DELETE_WINDOW", self.cancel)
        self.show()

    def default_target(self):
        """Next to the game: <Steam>/steamapps/common/ELDEN RING/DOOM RING (user); without ELDEN RING found,
        the user's Games folder."""
        if self.er_path:
            return Path(self.er_path) / "DOOM RING"
        return Path(os.environ.get("USERPROFILE", Path.home())) / "Games" / "DOOM RING"

    def checkbox(self, parent, text, var):
        """A check box with a real check mark (ttk's dark theme draws an X)."""
        row = ttk.Frame(parent)
        box = tk.Canvas(row, width=20, height=20, bg=BG, highlightthickness=0, cursor="hand2")
        box.pack(side="left")
        lab = ttk.Label(row, text=text, cursor="hand2")
        lab.pack(side="left", padx=(8, 0))

        def draw(*_):
            box.delete("all")
            on = var.get()
            box.create_rectangle(2, 2, 18, 18, outline=RED if on else DIM, width=2, fill=RED if on else PANEL)
            if on:
                box.create_line(5, 10, 9, 14, 15, 6, fill="white", width=2.5, capstyle="round", joinstyle="round")

        def toggle(_e=None):
            var.set(not var.get())

        box.bind("<Button-1>", toggle)
        lab.bind("<Button-1>", toggle)
        var.trace_add("write", draw)
        draw()
        return row

    # ------------------------------------------------------------------ navigation
    def clear(self):
        for w in self.body.winfo_children():
            w.destroy()

    def show(self):
        self.clear()
        self.back_btn.state(["!disabled"] if 0 < self.page < 3 else ["disabled"])
        self.next_btn.state(["!disabled"])
        self.next_btn.configure(text="Next >")
        self.pages[self.page]()

    def next(self):
        if self.page == 2:
            if not self.location_ok():
                return
        if self.page == len(self.pages) - 1:
            self.destroy()
            return
        self.page += 1
        self.show()

    def back(self):
        if self.page > 0:
            self.page -= 1
            self.show()

    def cancel(self):
        if self.installer and self.page == 3:
            if not messagebox.askyesno("DOOM RING Setup", "Stop the setup? The mod will not be complete."):
                return
            self.installer.cancel()
        self.destroy()

    def text(self, parent, s, style="TLabel", wrap=680, **kw):
        lab = ttk.Label(parent, text=s, style=style, wraplength=wrap, justify="left")
        lab.pack(anchor="w", **kw)
        return lab

    # ------------------------------------------------------------------ pages
    def page_welcome(self):
        ttk.Label(self.body, text=f"DOOM RING Version {self.version}", style="Title.TLabel").pack(anchor="w")
        self.text(self.body, "DOOM Eternal's guns, HUD, sounds and music inside ELDEN RING.", pady=(2, 14))
        self.text(self.body, (
            "This setup builds the mod on your PC from YOUR OWN copy of DOOM Eternal - no DOOM files come with "
            "the download. You need both games installed through Steam (base games, no DLC needed), about "
            "3 GB of free space, and 5 - 20 minutes depending on your PC."), pady=(0, 12))
        warn = ttk.Frame(self.body, style="Panel.TFrame", padding=14)
        warn.pack(fill="x", pady=(0, 12))
        ttk.Label(warn, text="Heads up: command windows", style="Panel.TLabel",
                  font=("Segoe UI", 10, "bold"), foreground=WARN).pack(anchor="w")
        ttk.Label(warn, style="Panel.TLabel", wraplength=660, justify="left", text=(
            "While it works, the setup runs small helper tools that read DOOM Eternal's files. You may see black "
            "command windows open or flash for a moment, and the game later starts through one (the mod loader). "
            "That is normal and safe - please don't close them; they close by themselves.")).pack(anchor="w", pady=(4, 0))
        self.text(self.body, (
            "DOOM RING runs ELDEN RING offline (Easy Anti-Cheat off, as every mod needs) on its own save file - "
            "your normal ELDEN RING save is never touched."), style="Dim.TLabel")
        self.next_btn.configure(text="Next >")

    def page_games(self):
        ttk.Label(self.body, text="Your games", style="Title.TLabel").pack(anchor="w", pady=(0, 12))
        er_ok, er_msg = engine.check_elden_ring(self.er_path)
        de_ok, de_msg = engine.check_doom(self.doom_path)
        for name, ok, msg, which in (("ELDEN RING", er_ok, er_msg, "er"), ("DOOM Eternal", de_ok, de_msg, "doom")):
            box = ttk.Frame(self.body, style="Panel.TFrame", padding=14)
            box.pack(fill="x", pady=(0, 10))
            head = ttk.Frame(box, style="Panel.TFrame")
            head.pack(fill="x")
            ttk.Label(head, text=("✔ " if ok else "✖ ") + name, style="Panel.TLabel", font=("Segoe UI", 12, "bold"),
                      foreground=GREEN if ok else RED).pack(side="left")
            ttk.Button(head, text="Browse...", command=lambda w=which: self.browse_game(w)).pack(side="right")
            ttk.Label(box, text=msg, style="Panel.TLabel", wraplength=640, justify="left",
                      foreground=FG if ok else WARN).pack(anchor="w", pady=(6, 0))
        row = ttk.Frame(self.body)
        row.pack(fill="x", pady=(4, 0))
        ttk.Button(row, text="Check again", command=self.recheck).pack(side="left")
        if not (er_ok and de_ok):
            self.text(row, "  Install the missing game(s) through Steam first, then press Check again.",
                      style="Dim.TLabel", side="left")
        self.next_btn.state(["!disabled"] if er_ok and de_ok else ["disabled"])

    def recheck(self):
        old = str(self.default_target())
        self.er_path = engine.find_app(engine.ELDEN_RING_APP) or self.er_path
        if self.target.get() == old:
            self.target.set(str(self.default_target()))
        self.doom_path = engine.find_app(engine.DOOM_ETERNAL_APP) or self.doom_path
        self.show()

    def browse_game(self, which):
        d = filedialog.askdirectory(title="ELDEN RING folder" if which == "er" else "DOOM Eternal folder")
        if d:
            if which == "er":
                old = str(self.default_target())
                self.er_path = Path(d)
                if self.target.get() == old:
                    self.target.set(str(self.default_target()))
            else:
                self.doom_path = Path(d)
            self.show()

    def page_location(self):
        ttk.Label(self.body, text="Where to put DOOM RING", style="Title.TLabel").pack(anchor="w", pady=(0, 12))
        self.text(self.body, ("The mod gets its own DOOM RING folder (about 1.5 GB) - by default next to the game, inside "
                              "your ELDEN RING folder. The game's own files are not changed."), pady=(0, 10))
        row = ttk.Frame(self.body)
        row.pack(fill="x")
        e = tk.Entry(row, textvariable=self.target, font=BODY_FONT, bg=PANEL, fg=FG, insertbackground=FG,
                     relief="flat")
        e.pack(side="left", fill="x", expand=True, ipady=6)
        ttk.Button(row, text="Browse...", command=self.browse_target).pack(side="left", padx=(8, 0))
        self.space = self.text(self.body, "", style="Dim.TLabel", pady=(8, 4))
        self.found = self.text(self.body, "", pady=(0, 10))
        self.found.configure(foreground=GREEN)
        self.update_space()
        self.checkbox(self.body, "Put a DOOM RING shortcut on the desktop", self.shortcut).pack(anchor="w")

    def existing(self):
        """(installed version, quick update?) for the chosen folder, or (None, False)."""
        try:
            old = engine.installed_version(self.target.get())
        except OSError:
            old = None
        if not old:
            return None, False
        return old["version"], old["content"] == engine.setup_version()["content"]

    def browse_target(self):
        d = filedialog.askdirectory(title="Folder for DOOM RING")
        if d:
            p = Path(d)
            self.target.set(str(p if p.name.lower() == "doom ring" else p / "DOOM RING"))

    def update_space(self):
        if not (self.space and self.space.winfo_exists()):     # (only on the location page)
            return
        try:
            gb = engine.free_gb(self.target.get())
            self.space.configure(text=f"Free space on that drive: {gb:.0f} GB (needs about 3 GB while installing)")
        except OSError:
            self.space.configure(text="")
        # an existing DOOM RING there is updated in place (settings, keys and progress are kept)
        old, quick = self.existing()
        self.updating = bool(old)
        if not old:
            self.found.configure(text="")
            self.next_btn.configure(text="Install")
            return
        if old == self.version:
            what = f"DOOM RING Version {old} is already installed here - Update puts its mod files back as new."
        else:
            what = f"DOOM RING Version {old} found here - Update brings it to Version {self.version}."
        what += (" Your settings, keys and progress are kept." + (" This takes a few seconds." if quick else
                 " The DOOM content is rebuilt too (5 - 20 minutes)."))
        self.found.configure(text="✔ " + what)
        self.next_btn.configure(text="Update")

    def location_ok(self):
        t = Path(self.target.get())
        if not t.is_absolute():
            messagebox.showerror("DOOM RING Setup", "Please choose a full folder path (for example C:\\Games\\DOOM RING).")
            return False
        try:
            if not self.existing()[1] and engine.free_gb(t) < 3:
                if not messagebox.askyesno("DOOM RING Setup", "That drive has less than 3 GB free. Try anyway?"):
                    return False
        except OSError:
            pass
        # Windows may not let us write there (Program Files without Steam's permissions)
        try:
            t.mkdir(parents=True, exist_ok=True)
            probe = t / "_write_test.tmp"
            probe.write_text("ok")
            probe.unlink()
        except OSError as e:
            messagebox.showerror("DOOM RING Setup", f"Windows won't let the setup write to\n{t}\n\n({e.strerror})\n\n"
                                 "Choose another folder, or start the setup with right-click > Run as administrator.")
            return False
        # (inside the ELDEN RING folder is fine - the default; just not in its Game folder or in DOOM Eternal)
        bad = [Path(p) for p in ([self.doom_path] if self.doom_path else [])]
        if self.er_path:
            bad.append(Path(self.er_path) / "Game")
        if any(str(t).lower().startswith(str(g).lower()) for g in bad):
            messagebox.showerror("DOOM RING Setup", "Please pick a folder outside DOOM Eternal and outside ELDEN RING's Game folder.")
            return False
        return True

    def page_install(self):
        ttk.Label(self.body, text="Updating DOOM RING" if self.updating else "Building DOOM RING", style="Title.TLabel").pack(anchor="w", pady=(0, 10))
        self.step_lbl = self.text(self.body, "Starting...")
        self.prog = ttk.Progressbar(self.body, style="red.Horizontal.TProgressbar", maximum=1000)
        self.prog.pack(fill="x", pady=(8, 10), ipady=3)
        self.text(self.body, "Command windows may flash while the helper tools run - that's normal.",
                  style="Dim.TLabel", pady=(0, 6))
        frame = ttk.Frame(self.body)
        frame.pack(fill="both", expand=True)
        self.logbox = tk.Text(frame, height=12, bg="#0e0c0a", fg=DIM, font=("Consolas", 9), relief="flat", wrap="none")
        sb = ttk.Scrollbar(frame, command=self.logbox.yview)
        self.logbox.configure(yscrollcommand=sb.set)
        sb.pack(side="right", fill="y")
        self.logbox.pack(fill="both", expand=True)
        self.back_btn.state(["disabled"])
        self.next_btn.state(["disabled"])
        self.next_btn.configure(text="Next >")
        self.installer = engine.Installer(self.doom_path, self.target.get(),
                                          on_progress=lambda f, s: self.after(0, self.progress, f, s),
                                          on_log=lambda line: self.after(0, self.add_log, line))
        engine.start_thread(self.run_install)

    def progress(self, frac, step):
        if frac is not None:
            self.prog["value"] = frac * 1000
        self.step_lbl.configure(text=step)

    def add_log(self, line):
        self.logbox.insert("end", line + "\n")
        if int(self.logbox.index("end-1c").split(".")[0]) > 4000:
            self.logbox.delete("1.0", "1000.0")
        self.logbox.see("end")

    def run_install(self):
        try:
            self.log_path = self.installer.install()
            if self.shortcut.get():
                engine.desktop_shortcut(self.target.get())
            self.after(0, self.finished, None)
        except engine.Cancelled:
            pass
        except Exception as e:  # noqa: BLE001
            self.after(0, self.finished, e)

    def finished(self, error):
        if error is None:
            self.page = 4
            self.show()
            return
        self.step_lbl.configure(text=f"Setup stopped: {error}", foreground=RED)
        self.add_log(f"ERROR: {error}")
        self.add_log(f"The full log is in {Path(self.target.get()) / 'setup_log.txt'}")
        self.cancel_btn.configure(text="Close")
        self.installer = None

    def page_done(self):
        ttk.Label(self.body, text=f"DOOM RING Version {self.version} is ready", style="Title.TLabel").pack(anchor="w", pady=(0, 12))
        self.text(self.body, f"{'Updated' if self.updating else 'Installed'} in {self.target.get()}", pady=(0, 10))
        self.text(self.body, (
            "Start it with \"Play DOOM RING\" (desktop shortcut or the .bat in the folder). ELDEN RING starts "
            "offline on the mod's own save. In game, F1 opens the DOOM RING settings."), pady=(0, 10))
        self.text(self.body, (
            "Controller players: in ELDEN RING's System > Camera settings, turn off \"Camera Auto Rotation\" and "
            "\"Auto Wall Recovery\" - the camera feels much better with the Doom movement."), style="Dim.TLabel",
            pady=(0, 14))
        row = ttk.Frame(self.body)
        row.pack(anchor="w")
        ttk.Button(row, text="Play DOOM RING", style="Accent.TButton", command=self.play).pack(side="left")
        ttk.Button(row, text="Open the folder", command=lambda: os.startfile(self.target.get())).pack(side="left", padx=8)
        self.back_btn.state(["disabled"])
        self.next_btn.configure(text="Finish")
        self.cancel_btn.state(["disabled"])

    def play(self):
        bat = Path(self.target.get()) / "Play DOOM RING.bat"
        subprocess.Popen(["cmd", "/c", str(bat)], cwd=str(bat.parent))
        self.destroy()


if __name__ == "__main__":
    Wizard().mainloop()
