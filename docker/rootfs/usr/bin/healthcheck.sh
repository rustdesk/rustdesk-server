#!/bin/sh

/package/admin/s6/command/s6-svstat /run/s6-rc/servicedirs/hbbr || exit 1
/package/admin/s6/command/s6-svstat /run/s6-rc/servicedirs/hbbs || exit 1
