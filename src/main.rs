//! Process (stereo) input and play the result (in stereo).

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SizedSample};
use fundsp::hacker32::*;

use crossbeam_channel::{bounded, Receiver, Sender};
use macroquad::prelude::*;

#[derive(Clone)]
pub struct InputNode {
    receiver: Receiver<(f32, f32)>,
}

impl InputNode {
    pub fn new(receiver: Receiver<(f32, f32)>) -> Self {
        InputNode { receiver }
    }
}

impl AudioNode for InputNode {
    const ID: u64 = 87;
    type Inputs = U0;
    type Outputs = U2;

    #[inline]
    fn tick(&mut self, _input: &Frame<f32, Self::Inputs>) -> Frame<f32, Self::Outputs> {
        let (left, right) = self.receiver.try_recv().unwrap_or((0.0, 0.0));
        [left, right].into()
    }
}

fn run_in<T>(device: &cpal::Device, config: &cpal::StreamConfig, sender: Sender<(f32, f32)>)
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let channels = config.channels as usize;
    let err_fn = |err| eprintln!("an error occurred on stream: {}", err);
    let stream = device.build_input_stream(
        config,
        move |data: &[T], _: &cpal::InputCallbackInfo| read_data(data, channels, sender.clone()),
        err_fn,
        None,
    );
    if let Ok(stream) = stream {
        if let Ok(()) = stream.play() {
            std::mem::forget(stream);
        }
    }
    println!("Input stream built.");
}

fn read_data<T>(input: &[T], channels: usize, sender: Sender<(f32, f32)>)
where
    T: SizedSample,
    f32: FromSample<T>,
{
    for frame in input.chunks(channels) {
        let mut left = 0.0;
        let mut right = 0.0;
        for (channel, sample) in frame.iter().enumerate() {
            if channel & 1 == 0 {
                left = sample.to_sample::<f32>();
            } else {
                right = sample.to_sample::<f32>();
            }
        }
        if let Ok(()) = sender.try_send((left, right)) {}
    }
}

fn run_out<T>(device: &cpal::Device, config: &cpal::StreamConfig, receiver: Receiver<(f32, f32)>)
where
    T: SizedSample + FromSample<f32>,
{
    let channels = config.channels as usize;

    let input = An(InputNode::new(receiver));
    let reverb = reverb2_stereo(20.0, 3.0, 1.0, 0.2, highshelf_hz(1000.0, 1.0, db_amp(-1.0)));
    let chorus = chorus(0, 0.0, 0.03, 0.2) | chorus(1, 0.0, 0.03, 0.2);
    // Here is the final input-to-output processing chain.
    let graph = input;
    let mut graph = BlockRateAdapter::new(Box::new(graph));
    graph.set_sample_rate(config.sample_rate.0 as f64);

    let mut next_value = move || graph.get_stereo();

    let err_fn = |err| eprintln!("An error occurred on stream: {}", err);
    let stream = device.build_output_stream(
        config,
        move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
            write_data(data, channels, &mut next_value)
        },
        err_fn,
        None,
    );
    if let Ok(stream) = stream {
        if let Ok(()) = stream.play() {
            std::mem::forget(stream);
        }
    }
    println!("Output stream built.");
}

fn write_data<T>(output: &mut [T], channels: usize, next_sample: &mut dyn FnMut() -> (f32, f32))
where
    T: SizedSample + FromSample<f32>,
{
    for frame in output.chunks_mut(channels) {
        let sample = next_sample();
        let left = T::from_sample(sample.0);
        let right = T::from_sample(sample.1);

        for (channel, sample) in frame.iter_mut().enumerate() {
            if channel & 1 == 0 {
                *sample = left;
            } else {
                *sample = right;
            }
        }
    }
}

#[macroquad::main("3D")]
async fn main() {
    let mut player_pos = vec3(-8., 0., 0.);
    loop {
        clear_background(LIGHTGRAY);

        // Camera
        set_camera(&Camera3D {
            position: vec3(-20., 15., 0.),
            up: vec3(0., 1., 0.),
            target: vec3(0., 0., 0.),
            ..Default::default()
        });

        draw_grid(20, 1., BLACK, GRAY);

        /* Source */ 
        draw_sphere(vec3(0., 0., 0.), 1., None, BLACK);

        /* Player */
        if is_key_down(KeyCode::Left) {
            player_pos.z -= 0.1;
        }
        if is_key_down(KeyCode::Right) {
            player_pos.z += 0.1;
        }
        if is_key_down(KeyCode::Up) {
            player_pos.x += 0.1;
        }
        if is_key_down(KeyCode::Down) {
            player_pos.x -= 0.1;
        }

        draw_sphere(player_pos, 1., None, BLUE);

        // Back to screen space, render some text

        set_default_camera();
        draw_text("WELCOME TO 3D WORLD", 10.0, 20.0, 30.0, BLACK);

        next_frame().await
    }
}
