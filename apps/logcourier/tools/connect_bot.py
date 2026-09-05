"""Interactive initial setup. Token input is hidden, never a command argument."""

from getpass import getpass

from logcourier.config import data_directory, load_config, save_config
from logcourier.secrets import redact, store_token
from logcourier.telegram import Telegram


def main():
    token = getpass("Telegram bot token (hidden): ")
    try:
        client = Telegram(token)
        me = client.call("getMe")
        if str(me["id"]) != client.bot_id or not me.get("is_bot"):
            raise ValueError("Telegram не подтвердил бота")
        store_token(token)
        root = data_directory()
        config = load_config(root)
        if config.bot_id != client.bot_id:
            config.bot_id = client.bot_id
            config.consent = False
            config.auto_send = False
            config.chat_id = ""
        save_config(root, config)
        print(f"Bot verified: @{me['username']}; ID {client.bot_id}")
        print("Token saved in OS keyring. No logs sent.")
        groups = client.groups()
        for group in groups:
            print(f"Group: {group['id']} — {group.get('title', '')}")
        if not groups:
            print("No recent groups found. Add the bot and send /start@bot_username in the group.")
        return 0
    except Exception as error:
        print(
            redact(str(error))
            if isinstance(error, (ValueError, RuntimeError))
            else f"Setup failed ({type(error).__name__})"
        )
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
