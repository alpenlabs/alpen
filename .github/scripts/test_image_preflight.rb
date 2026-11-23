require 'minitest/autorun'
require 'tmpdir'
require 'yaml'
require 'json'
require 'open3'

class ImagePreflightTest < Minitest::Test
  def test_build_reuse_backfill_and_conflict
    steps = YAML.load_file(File.expand_path('../workflows/ci-build.yml', __dir__)).fetch('jobs').fetch('build-and-push').fetch('steps')
    script = steps.find { |step| step['id'] == 'images' }.fetch('run')
    Dir.mktmpdir do |dir|
      mock = File.join(dir, 'skopeo')
      File.write(mock, <<~'SH')
        #!/bin/bash
        if [[ "$1" == copy ]]; then
          [[ "$2 $3" == '--all --preserve-digests' && "$4" == *@sha256:* ]] || exit 1
          echo "$5" >> "$RUNNER_TEMP/copied"
        elif jq -e --arg image "$3" 'has($image)' <<< "$REGISTRY" >/dev/null; then
          jq --arg image "$3" '{Digest:.[$image],Labels:{"org.opencontainers.image.revision":"full"}}' <<< "$REGISTRY"
        else
          echo 'manifest unknown: Requested image not found' >&2
          exit 1
        fi
      SH
      File.chmod(0o755, mock)
      commit = 'private/prefix/strata:aaaaaaa'
      mirror = 'public.ecr.aws/alpenlabs/strata:aaaaaaa'
      digest = 'sha256:' + '1' * 64
      version = 'v1.0.0-rc.1'
      aliases = [commit, mirror].map { |ref| ref.sub(':aaaaaaa', ":#{version}-aaaaaaa") }
      [[{}, 'true', []], [{commit => digest}, 'false', [mirror]],
       [{commit => digest, mirror => digest}, 'false', []],
       [{commit => digest, mirror => 'sha256:other'}, nil, []],
       [{commit => digest, mirror => digest}, 'false', aliases, version]].each do |registry, build, copies, release|
        output = File.join(dir, 'outputs')
        copied = File.join(dir, 'copied')
        [output, copied].each { |path| File.write(path, '') }
        env = {'PATH' => "#{dir}:#{ENV.fetch('PATH')}", 'RUNNER_TEMP' => dir, 'GITHUB_OUTPUT' => output,
               'PRIVATE_REPO' => 'private/prefix/strata',
               'SHORT_TAG' => 'aaaaaaa', 'RELEASE_TAG' => release.to_s, 'COMMIT_SHA' => 'full',
               'PUBLIC_REPO' => 'strata', 'PUSH_PRIVATE' => 'true',
               'REGISTRY' => JSON.generate(registry.transform_keys { |key| "docker://#{key}" })}
        log, status = Open3.capture2e(env, 'bash', '-eo', 'pipefail', '-c', script)
        assert_equal !build.nil?, status.success?, log
        assert_equal(build == 'true' ? "build_tags=#{commit} #{mirror}\n" : '', File.read(output))
        assert_equal copies.map { |image| "docker://#{image}" }, File.readlines(copied, chomp: true)
      end
    end
  end
end
