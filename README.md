### RustDesk | Your Remote Desktop Software

This is a repository used to release RustDesk server software and track issues.

Built on Centos7, tested on Centos7/8, Ubuntu 18/20.

There are two executables
  - hbbs - RustDesk ID/Rendezvous server
  - hbbr - RustDesk relay server

By default, hbbs listens on 21115(tcp) and 21116(tcp/udp), hbbr listens on 21117(tcp)

[check here](https://rustdesk.com/blog/id-relay-set/) for more information about setting up your cloud with hbbs/hbbr.

For sustainable development, this software is no longer available for free. Please pay USD $99 to my paypal **info@rustdesk.com**. Then send email to info@rustdesk.com to confirm the payment, and I will reply you with the program.

> You can write your own rustdesk-server, the protocol is [open sourced](https://github.com/rustdesk/rustdesk/tree/master/src/rendezvous_mediator.rs). You just need some time to understand, instead you can buy me a cup of tea, **:)**
