---
title: dex documentation
description: Documentation for dex, a Raspberry Pi video player that loops an artwork seamlessly.
template: splash
hero:
  tagline: A Raspberry Pi plays one video in a gapless loop, on the screen or projector in the room.
  actions:
    - text: Configure the exhibit
      link: /guides/configure-exhibit/
      icon: right-arrow
    - text: Source on GitHub
      link: https://github.com/KTE/dex
      icon: external
      variant: minimal
---

## Guides

For the person setting up the player. A terminal and a recipe are enough; no knowledge of video
codecs, Linux graphics or Rust is assumed.

- [Configure the exhibit](/guides/configure-exhibit/) — the one file that says which video plays
  and how the display is driven.

## Design

For a developer reading how the player works.

- [Why an endless stream](/design/endless-stream/) — how dexd loops a video with no held frame.

## Reference

- [Glossary](/glossary/) — every term the documentation uses without explaining it.
