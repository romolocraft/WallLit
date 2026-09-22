use windows::core::*;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;

pub const GEAR: &str = "M259.1 73.5C262.1 58.7 275.2 48 290.4 48L350.2 48C365.4 48 378.5 58.7 381.5 73.5L396 143.5C410.1 149.5 423.3 157.2 435.3 166.3L503.1 143.8C517.5 139 533.3 145 540.9 158.2L570.8 210C578.4 223.2 575.7 239.8 564.3 249.9L511 297.3C511.9 304.7 512.3 312.3 512.3 320C512.3 327.7 511.8 335.3 511 342.7L564.4 390.2C575.8 400.3 578.4 417 570.9 430.1L541 481.9C533.4 495 517.6 501.1 503.2 496.3L435.4 473.8C423.3 482.9 410.1 490.5 396.1 496.6L381.7 566.5C378.6 581.4 365.5 592 350.4 592L290.6 592C275.4 592 262.3 581.3 259.3 566.5L244.9 496.6C230.8 490.6 217.7 482.9 205.6 473.8L137.5 496.3C123.1 501.1 107.3 495.1 99.7 481.9L69.8 430.1C62.2 416.9 64.9 400.3 76.3 390.2L129.7 342.7C128.8 335.3 128.4 327.7 128.4 320C128.4 312.3 128.9 304.7 129.7 297.3L76.3 249.8C64.9 239.7 62.3 223 69.8 209.9L99.7 158.1C107.3 144.9 123.1 138.9 137.5 143.7L205.3 166.2C217.4 157.1 230.6 149.5 244.6 143.4L259.1 73.5zM320.3 400C364.5 399.8 400.2 363.9 400 319.7C399.8 275.5 363.9 239.8 319.7 240C275.5 240.2 239.8 276.1 240 320.3C240.2 364.5 276.1 400.2 320.3 400z";

pub const STAR: &str = "M320 64L397.5 221.1L570.9 246.3L445.5 368.6L475.1 541.3L320 459.8L164.9 541.3L194.5 368.6L69.1 246.3L242.5 221.1L320 64z";

pub const PLUS: &str =
    "M336 160L336 288L464 288L464 352L336 352L336 480L272 480L272 352L144 352L144 288L272 288L272 160L336 160z";

pub struct Icon {
    geometry: ID2D1PathGeometry1,

    viewbox: f32,
}

impl Icon {
    pub fn new(factory: &ID2D1Factory1, path: &str, viewbox: f32) -> Result<Self> {
        let geometry = unsafe { factory.CreatePathGeometry() }?;
        let sink = unsafe { geometry.Open() }?;

        unsafe { sink.SetFillMode(D2D1_FILL_MODE_ALTERNATE) };

        build(&sink, path);

        unsafe { sink.Close() }?;
        Ok(Self { geometry, viewbox })
    }

    pub fn geometry(&self) -> &ID2D1PathGeometry1 {
        &self.geometry
    }

    pub fn viewbox(&self) -> f32 {
        self.viewbox
    }

    pub fn transform(&self, origin: (f32, f32), size: f32) -> Matrix3x2 {
        let scale = size / self.viewbox;
        Matrix3x2 {
            M11: scale,
            M12: 0.0,
            M21: 0.0,
            M22: scale,
            M31: origin.0,
            M32: origin.1,
        }
    }
}

fn build(sink: &ID2D1GeometrySink, path: &str) {
    let mut cursor = Cursor::new(path);
    let mut open = false;
    let mut start = D2D_POINT_2F::default();
    let mut current = D2D_POINT_2F::default();

    while let Some(command) = cursor.command() {
        match command {
            'M' => {
                if open {
                    unsafe { sink.EndFigure(D2D1_FIGURE_END_OPEN) };
                }
                let Some(point) = cursor.point() else { break };
                unsafe { sink.BeginFigure(point, D2D1_FIGURE_BEGIN_FILLED) };
                start = point;
                current = point;
                open = true;
            }

            'L' => {
                while let Some(point) = cursor.point() {
                    unsafe { sink.AddLine(point) };
                    current = point;
                }
            }

            'C' => {
                while let (Some(one), Some(two), Some(three)) =
                    (cursor.point(), cursor.point(), cursor.point())
                {
                    let segment = D2D1_BEZIER_SEGMENT {
                        point1: one,
                        point2: two,
                        point3: three,
                    };
                    unsafe { sink.AddBezier(&segment) };
                    current = three;
                }
            }

            'Z' | 'z' => {
                if open {
                    unsafe { sink.EndFigure(D2D1_FIGURE_END_CLOSED) };
                    open = false;
                }
                current = start;
            }

            _ => break,
        }
    }

    let _ = current;
    if open {
        unsafe { sink.EndFigure(D2D1_FIGURE_END_OPEN) };
    }
}

struct Cursor<'a> {
    bytes: &'a [u8],
    position: usize,

    pending: Option<char>,
}

impl<'a> Cursor<'a> {
    fn new(path: &'a str) -> Self {
        Self { bytes: path.as_bytes(), position: 0, pending: None }
    }

    fn skip_separators(&mut self) {
        while self.position < self.bytes.len() {
            match self.bytes[self.position] {
                b' ' | b',' | b'\t' | b'\n' | b'\r' => self.position += 1,
                _ => break,
            }
        }
    }

    fn command(&mut self) -> Option<char> {
        self.skip_separators();
        let byte = *self.bytes.get(self.position)?;

        if byte.is_ascii_alphabetic() {
            self.position += 1;
            let command = byte as char;

            self.pending = Some(if command == 'M' { 'L' } else { command });
            return Some(command);
        }

        self.pending
    }

    fn point(&mut self) -> Option<D2D_POINT_2F> {
        let x = self.number()?;
        let y = self.number()?;
        Some(D2D_POINT_2F { x, y })
    }

    fn number(&mut self) -> Option<f32> {
        self.skip_separators();

        let start = self.position;
        if matches!(self.bytes.get(self.position), Some(b'-') | Some(b'+')) {
            self.position += 1;
        }

        while matches!(self.bytes.get(self.position), Some(byte) if byte.is_ascii_digit() || *byte == b'.')
        {
            self.position += 1;
        }

        if self.position == start {
            return None;
        }

        std::str::from_utf8(&self.bytes[start..self.position])
            .ok()?
            .parse()
            .ok()
    }
}

pub type Matrix3x2 = windows::Foundation::Numerics::Matrix3x2;
