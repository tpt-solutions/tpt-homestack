# Signing in as a family member

This is for the person who runs the server (the admin) and for each family member
who will use it. Plain language, and no terminal is needed after setup.

## For the admin: adding someone

1. Sign in as the admin and open **Admin · Users** from the sidebar.
2. In **Create user**, type the person's name, a temporary password of at least
   8 characters, and leave the role as **user**. Choose **admin** only for someone
   who should manage accounts and plugins.
3. Give the person the name and the temporary password in person or by a private
   message. Do not send it in a shared chat.
4. Tell them the address to open (see "Opening it on a phone" below).

The temporary password is only for the first sign-in. The app makes them choose
their own password before anything else works.

If someone forgets their password, open **Admin · Users**, use **Reset pw**, and
give them the new temporary password. They will be asked to change it again.

## For the family member: first sign-in

1. Open the address you were given in a browser (on a phone, tablet or computer).
2. Enter your username and the temporary password, then **Sign in**.
3. You will be asked to choose a new password. Use at least 8 characters, and
   make it one you do not use anywhere else.
4. You are now on the dashboard. You can use the apps the admin has turned on.
   The Admin pages are for the admin only.

On a phone, you can add the page to your home screen from the browser's menu
(the wording differs between iPhone and Android).

## Opening it on a phone or tablet

- **At home (same Wi-Fi):** open the address shown under **Settings → Open on a
  phone or tablet**, or scan the code on that page. The name works on most phones
  and computers; if it does not, ask the admin for the server's IP address and
  port.
- **Away from home:** this needs the remote-access setup in
  [REMOTE_ACCESS.md](REMOTE_ACCESS.md), which the admin arranges. Tailscale is the
  simplest choice for families: install its app, sign in with the account the admin
  invited you to, and open the address the admin gives you.

## Leaving the family, or losing a phone

- Ask the admin to **disable** your account (Admin · Users, **Disable**). Your
  sessions stop working straight away.
- If you use Tailscale, the admin can also remove your device there.
- If a phone is lost, sign out of the app on it, and tell the admin at once.
