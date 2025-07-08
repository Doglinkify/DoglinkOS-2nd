#[allow(unused_mut)]
fn main() {
    println!("Hello from rust std");
    let mut vec1 = vec![1, 2, 3, 73];
    // vec1.reserve_exact(1);
    // vec1.reserve_exact(1);
    // vec1.reserve_exact(1);
    // vec1.reserve_exact(1);
    for value in vec1 {
        println!("got element {value}");
    }
    let box1 = Box::new([1324, 1432, 2342]);
    println!("boxed {box1:?}");
    unsafe {
        std::arch::asm!(
            "int 0x80",
            in("rax") 3,
            in("rdi") "/sbin/doglinked".as_ptr(),
            in("rcx") "/sbin/doglinked".len(),
        );
    }
}
