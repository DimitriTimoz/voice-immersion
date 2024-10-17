use cpal::traits::{DeviceTrait, HostTrait};
use crossbeam_channel::bounded;
use fundsp::wave::Wave;
use macroquad::prelude::*;
use std::sync::{Arc, RwLock};
use voice_immersion::{run_in, run_out, InAnotherRoom, SourceInfo, HEAD_RADIUS};

#[macroquad::main("3D")]
async fn main() -> anyhow::Result<()> {
    let source_info = Arc::new(RwLock::new(SourceInfo::default()));
    let source_info_audio = source_info.clone();
    std::thread::spawn(move || {
        // Sender / receiver for left and right channels (stereo mic).
        let (sender, receiver) = bounded(4096);

        let host = cpal::default_host();
        // Start input.
        let in_device = host.default_input_device().unwrap();
        let in_config = in_device.default_input_config().unwrap();
        let wave = Wave::load("loop3.flac").unwrap();

        match in_config.sample_format() {
            cpal::SampleFormat::F32 => run_in::<f32>(&in_device, &in_config.into(), sender),
            cpal::SampleFormat::I16 => run_in::<i16>(&in_device, &in_config.into(), sender),
            cpal::SampleFormat::U16 => run_in::<u16>(&in_device, &in_config.into(), sender),
            format => eprintln!("Unsupported sample format: {}", format),
        }
        // Start output.
        let out_device = host.default_output_device().unwrap();
        let out_config = out_device.default_output_config().unwrap();
        match out_config.sample_format() {
            cpal::SampleFormat::F32 => {
                let _ = run_out::<f32>(
                    &out_device,
                    &out_config.into(),
                    receiver,
                    wave,
                    source_info_audio,
                );
            }
            cpal::SampleFormat::I16 => {
                let _ = run_out::<i16>(
                    &out_device,
                    &out_config.into(),
                    receiver,
                    wave,
                    source_info_audio,
                );
            }
            cpal::SampleFormat::U16 => {
                let _ = run_out::<u16>(
                    &out_device,
                    &out_config.into(),
                    receiver,
                    wave,
                    source_info_audio,
                );
            }
            format => eprintln!("Unsupported sample format: {}", format),
        }
    });

    println!("Processing stereo input to stereo output.");

    let mut player_pos = vec3(-2., 0., 0.);

    loop {
        clear_background(LIGHTGRAY);

        // Camera
        set_camera(&Camera3D {
            position: vec3(-10., 5., 0.),
            up: vec3(0., 1., 0.),
            target: vec3(0., 0., 0.),
            ..Default::default()
        });

        draw_grid(10, 1., BLACK, GRAY);

        /* Source */
        draw_sphere(vec3(0., 0., 0.), HEAD_RADIUS, None, BLACK);

        /* Player */
        if is_key_down(KeyCode::Left) {
            player_pos.z -= 0.01;
        }
        if is_key_down(KeyCode::Right) {
            player_pos.z += 0.01;
        }
        if is_key_down(KeyCode::Up) {
            player_pos.x += 0.01;
        }
        if is_key_down(KeyCode::Down) {
            player_pos.x -= 0.01;
        }

        let direction = vec3(1., 0., 0.);
        // Draw Room
        draw_cube_wires(vec3(-1.5, 0., 0.0), vec3(1., 0., 01.), YELLOW);

        let in_room =
            player_pos.x > -2.0 && player_pos.x < -1.0 && player_pos.z > -0.5 && player_pos.z < 0.5;

        draw_sphere(player_pos, HEAD_RADIUS, None, BLUE);
        draw_line_3d(player_pos, player_pos + direction, RED);
        set_default_camera();
        draw_text(
            &format!("Player pos in room: {}", in_room),
            10.0,
            20.0,
            30.0,
            BLACK,
        );

        if let Ok(mut source_info) = source_info.try_write() {
            source_info.relative_position.x = -player_pos.x;
            source_info.relative_position.y = -player_pos.y;
            source_info.relative_position.z = -player_pos.z;
            source_info.direction.x = direction.x;
            source_info.direction.y = direction.y;
            source_info.direction.z = direction.z;
            source_info.room = if in_room {
                Some(InAnotherRoom {
                    wall_attenuation_factor: 500.,
                    wall_width: 0.002,
                    cutoff_frequency: 2000.,
                })
            } else {
                None
            };
        }

        next_frame().await
    }
}
