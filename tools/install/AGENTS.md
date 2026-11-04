The packaged directory is the pre-publication stand-in for crates.io.
Keep installed-binary assertions shared with the binary e2e suite.
This test target is run by the separate CI install job, outside the development
gate, so packaging and installation run once per platform.
