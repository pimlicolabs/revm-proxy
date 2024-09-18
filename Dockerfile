# Use the official Rust image as the base image
FROM rust:latest

# Set the working directory inside the container
WORKDIR /usr/src/app

# Install libclang and other necessary dependencies
RUN apt-get update && apt-get install -y clang

# Copy the project files
COPY . .

# Build the project
RUN cargo build --release

# Run the compiled binary
CMD ["./target/release/revm-passthrough-proxy"]
