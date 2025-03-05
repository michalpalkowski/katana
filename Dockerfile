FROM ghcr.io/dojoengine/dojo-dev:v1.2.2

# Set environment variables for LLVM
ENV MLIR_SYS_190_PREFIX=/usr/lib/llvm-19
ENV	LLVM_SYS_191_PREFIX=/usr/lib/llvm-19
ENV TABLEGEN_190_PREFIX=/usr/lib/llvm-19

# Add LLVM 19 repository
RUN wget -O - https://apt.llvm.org/llvm-snapshot.gpg.key | apt-key add - \
	&& echo "deb http://apt.llvm.org/jammy/ llvm-toolchain-jammy-19 main" >> /etc/apt/sources.list.d/llvm-19.list \
	&& apt-get update

# Install LLVM and Cairo native dependencies
RUN apt-get install -y \
	llvm-19 \
	llvm-19-dev \
	llvm-19-runtime \
	clang-19 \
	clang-tools-19 \
	lld-19 \
	libpolly-19-dev \
	libmlir-19-dev \
	mlir-19-tools

# Install pyenv for SNOS artifact build script
RUN curl https://pyenv.run | bash
# Workaround for https://github.com/actions/runner-images/issues/6775
RUN git config --global --add safe.directory "*"
