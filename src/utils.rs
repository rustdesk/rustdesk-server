use sodiumoxide::crypto::sign;
use std::str;
use std::env;
use std::process;

fn print_help() {
    println!("Usage:");
    println!("  rustdesk-util [command]\n");
    println!("Available Commands:");
    println!("  genkeypair                                   Generate a new keypair");
    println!("  validatekeypair [public key] [secret key]    Validate an existing keypair");
    process::exit(0x0001);
}

fn error_then_help(msg: &str) {
    println!("ERROR: {}\n", msg);
    print_help();
}

fn gen_keypair() {
    let (pk, sk) = sign::gen_keypair();
    let public_key = base64::encode(pk);
    let secret_key = base64::encode(sk);
    println!("Public Key:  {public_key}");
    println!("Secret Key:  {secret_key}");
}

fn validate_keypair(pk: &str, sk: &str) {
    let sk1 = base64::decode(&sk);
    match sk1 {
        Ok(_) => {},
        Err(_) => {
            println!("Invalid secret key");
            process::exit(0x0001);
        },
    }
    let sk1 = sk1.unwrap();

    let secret_key = sign::SecretKey::from_slice(sk1.as_slice());
    match secret_key {
        Some(_) => {},
        None => {
            println!("Invalid Secret key");
            process::exit(0x0001);
        },
    }
    let secret_key = secret_key.unwrap();

    let pk1 = base64::decode(&pk);
    match pk1 {
        Ok(_) => {},
        Err(_) => {
            println!("Invalid public key");
            process::exit(0x0001);
        },
    }
    let pk1 = pk1.unwrap();

    let public_key = sign::PublicKey::from_slice(pk1.as_slice());
    match public_key {
        Some(_) => {},
        None => {
            println!("Invalid Public key");
            process::exit(0x0001);
        },
    }
    let public_key = public_key.unwrap();

    let random_data_to_test = b"This is meh.";
    let signed_data = sign::sign(random_data_to_test, &secret_key);
    let verified_data = sign::verify(&signed_data, &public_key);
    match verified_data {
        Ok(_) => {},
        Err(_) => {
            println!("Key pair is INVALID");
            process::exit(0x0001);
        },
    }
    let verified_data = verified_data.unwrap();

    if random_data_to_test == &verified_data[..] {
        println!("Key pair is VALID");
    } else {
        println!("Key pair is INVALID");
        process::exit(0x0001);
    }
}

fn main() {
    let args: Vec<_> = env::args().collect();
    if args.len() <= 1 {
        print_help();
    }

    let command = args[1].to_lowercase();
    match command.as_str() {
        "genkeypair" => gen_keypair(),
        "validatekeypair" => {
            if args.len() <= 3 {
                error_then_help("You must supply both the public and the secret key");
            }
            validate_keypair(args[2].as_str(),args[3].as_str());
        },
        _=>print_help(),
    }
}