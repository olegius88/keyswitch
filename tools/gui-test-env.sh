# Source only from test runners, never from the application or login profile.
# GTK4 does not use GTK_USE_PORTAL=0 to disable its settings portal.
# https://docs.gtk.org/gtk4/running.html#gdk-debug
export GDK_DEBUG="${GDK_DEBUG:+$GDK_DEBUG,}no-portals"
# Libadwaita has its own portal client, independent of GTK/GDK settings.
export ADW_DISABLE_PORTAL=1
export GTK_USE_PORTAL=0
export GIO_USE_VFS=local
export GDK_BACKEND=x11
# GTK draws with cairo unless told otherwise: inside Xvfb the Vulkan renderer on an NVIDIA
# driver lost its device on 2026-10-14 and logged one warning for 25 minutes, 37 GB. The
# tests check input and widgets, not rendering.
# https://docs.gtk.org/gtk4/running.html#gsk_renderer
export GSK_RENDERER="${GSK_RENDERER:-cairo}"
