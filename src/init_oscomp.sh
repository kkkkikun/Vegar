#!/bin/sh
echo "=== StarryOS OSComp Quick Test ==="

# --- basic-musl ---
cd /musl
echo "#### OS COMP TEST GROUP START basic-musl ####"
cd basic
for t in brk chdir clone close dup dup2 execve exit fork fstat getcwd getdents getpid getppid gettimeofday mkdir_ mmap mount munmap open openat pipe read sleep times umount uname unlink wait waitpid write yield
do
  echo "Testing $t :"
  ./$t
done
cd ..
echo "#### OS COMP TEST GROUP END basic-musl ####"

# --- basic-glibc ---
# glibc binaries need:
#   /lib/ld-linux-riscv64-lp64d.so.1  (served by kernel virtual /lib)
#   libc.so.6, libm.so.6               (on ext4 at /glibc/lib/)
# LD_LIBRARY_PATH tells ld-linux where to find the shared libraries.
echo ""
cd /glibc
export LD_LIBRARY_PATH=/glibc/lib
echo "#### OS COMP TEST GROUP START basic-glibc ####"
cd basic
for t in brk chdir clone close dup dup2 execve exit fork fstat getcwd getdents getpid getppid gettimeofday mkdir_ mmap mount munmap open openat pipe read sleep times umount uname unlink wait waitpid write yield
do
  echo "Testing $t :"
  ./$t
done
cd ..
unset LD_LIBRARY_PATH
echo "#### OS COMP TEST GROUP END basic-glibc ####"

# --- libctest-musl smoke ---
# libctest uses runtest.exe (static) as a harness that spawns entry-*.exe.
# entry-static.exe is static, entry-dynamic.exe needs ld-musl (already in /lib).
echo ""
cd /musl
echo "#### OS COMP TEST GROUP START libctest-musl ####"
echo "--- libctest-musl: static tests (first 5) ---"
./runtest.exe -w entry-static.exe argv
./runtest.exe -w entry-static.exe basename
./runtest.exe -w entry-static.exe clock_gettime
./runtest.exe -w entry-static.exe dirname
./runtest.exe -w entry-static.exe env
echo "--- libctest-musl: dynamic tests (first 5) ---"
./runtest.exe -w entry-dynamic.exe argv
./runtest.exe -w entry-dynamic.exe basename
./runtest.exe -w entry-dynamic.exe clock_gettime
./runtest.exe -w entry-dynamic.exe dirname
./runtest.exe -w entry-dynamic.exe env
echo "#### OS COMP TEST GROUP END libctest-musl ####"

# --- busybox-musl full ---
echo ""
cd /musl
./busybox sh ./busybox_testcode.sh

echo "=== OSComp Quick Test Done ==="
