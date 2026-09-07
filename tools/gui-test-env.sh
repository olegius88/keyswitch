# Source only from test runners, never from the application or login profile.
# GTK4 does not use GTK_USE_PORTAL=0 to disable its settings portal.
# https://docs.gtk.org/gtk4/running.html#gdk-debug
export GDK_DEBUG="${GDK_DEBUG:+$GDK_DEBUG,}no-portals"
# Libadwaita has its own portal client, independent of GTK/GDK settings.
export ADW_DISABLE_PORTAL=1
export GTK_USE_PORTAL=0
export GIO_USE_VFS=local
export GDK_BACKEND=x11
