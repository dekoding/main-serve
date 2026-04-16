# Manifesto (Or: Why Does This Thing Exist?)

It was 4:00 AM, and I was out of bed. I'd woken up with an idea for a useful application, and couldn't wait. So I'd hurried to my computer and started working on it.

After a few hours of head-down-and-fingers-on-keyboard coding, I ran into the same roadblock everyone runs into eventually.

**"This needs a backend."**

Indeed it did. So I context-switched, and started working on the design and implementation of a REST API that would serve as its source of data. I spent time thinking about what I'd write it in, how I'd organize it, how I'd configure the authentication layer, and all the other fiddly bits and boilerplate that... well, that took me away from the fun stuff I'd been working on.

After a while, I sat back and said to myself, "I wish I could just tell it what databases and tables to use and leave it at that, so I could get back to the interesting parts."

And suddenly, my amazing application idea moved itself to the backburner in my mind, replaced with this:

```yaml
server:
  host: "127.0.0.1"
  port: 8080

tables:
  ...

endpoints:
  - path: "/api/something"
  ...
```

That was it. I didn't know what to put there - what to replace `...` with - but I knew that if I could figure it out, I had something. Something genuinely and unequivocally **useful**.

I pictured myself working on my now nearly-forgotten application idea, only this time having something running that could serve up any CRUD operation I'd need without any effort on my part beyond simply telling it what database to use and what tables to give me.

And then I set to work.

## Purpose

**Main Serve** is intended to fulfill two purposes:

1. It replaces all the boilerplate and boring backend work of providing CRUD operations for a frontend. It won't replace all potential hand-made backend APIs - especially not those that have complex business logic or necessary side-effects. But for everyday, run-of-the-mill CRUD operations on a typical SQL database? Which probably describes 99% of API work out there? It can definitely replace those, especially early in the development lifecycle when all you need is a table of users.
2. Written in rust, it is fast and it is **stable**. Stable enough to serve as a production server that solves issues with CORS, hosts your static assets, and even provides pass-through proxy support for external APIs. Again, it won't solve complex serving needs, but it's a no-code solution for simpler use-cases.

I wrote this for myself, but I think it's good enough now to share with the world. I hope you find it just as useful.

*Damon Kaswell (dekoding)*
